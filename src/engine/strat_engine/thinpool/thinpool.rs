// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle management of a pool's thinpool device.

use std;
use std::borrow::BorrowMut;
use std::cmp::{max, min};
use std::fs::File;
use std::io::{BufRead, BufReader};
use uuid::Uuid;

use devicemapper::{
    device_exists, DataBlocks, Device, DmDevice, DmName, DmNameBuf, FlakeyTargetParams, LinearDev,
    LinearDevTargetParams, LinearTargetParams, MetaBlocks, Sectors, TargetLine, ThinDevId,
    ThinPoolDev, ThinPoolStatus, ThinPoolStatusSummary, IEC,
};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::devlinks;
use super::super::super::engine::Filesystem;
use super::super::super::event::{get_engine_listener_list, EngineEvent};
use super::super::super::structures::Table;
use super::super::super::types::{
    FilesystemUuid, FreeSpaceState, MaybeDbusPath, Name, PoolExtendState, PoolState, PoolUuid,
    RenameAction,
};

use super::super::backstore::Backstore;
use super::super::cmd::{thin_check, thin_repair};
use super::super::device::wipe_sectors;
use super::super::dm::get_dm;
use super::super::names::{
    format_flex_ids, format_thin_ids, format_thinpool_ids, FlexRole, ThinPoolRole, ThinRole,
};
use super::super::serde_structs::{FlexDevsSave, Recordable, ThinPoolDevSave};
use super::super::set_write_throttling;

use super::filesystem::{fs_settle, FilesystemStatus, StratFilesystem};
use super::mdv::MetadataVol;
use super::thinids::ThinDevIdPool;

pub const DATA_BLOCK_SIZE: Sectors = Sectors(2 * IEC::Ki);
pub const DATA_LOWATER: DataBlocks = DataBlocks(2048); // 2 GiB
const DATA_EXPAND_SIZE: Sectors = Sectors(16 * IEC::Mi); // 8 GiB
const META_LOWATER_FALLBACK: MetaBlocks = MetaBlocks(1024);

const INITIAL_META_SIZE: MetaBlocks = MetaBlocks(4 * IEC::Ki);
const INITIAL_DATA_SIZE: DataBlocks = DataBlocks(768);
const INITIAL_MDV_SIZE: Sectors = Sectors(32 * IEC::Ki); // 16 MiB

const SPACE_WARN_PCT: u8 = 90;
const SPACE_CRIT_PCT: u8 = 95;

/// When Stratis initiates throttling, this is the value it always specifies.
const THROTTLE_BLOCKS_PER_SEC: DataBlocks = DataBlocks(10);

fn sectors_to_datablocks(sectors: Sectors) -> DataBlocks {
    DataBlocks(sectors / DATA_BLOCK_SIZE)
}

fn datablocks_to_sectors(data_blocks: DataBlocks) -> Sectors {
    *data_blocks * DATA_BLOCK_SIZE
}

/// Get the value for the "Dirty" key in /proc/meminfo in Sectors.
/// Return an error if "Dirty" key is not found, value can not be parsed,
/// or there is some error reading /proc/meminfo.
fn current_dirty_mem() -> StratisResult<Sectors> {
    for line in BufReader::new(File::open("/proc/meminfo")?).lines() {
        let line = line?;
        let mut iter = line.split_whitespace();
        if iter.next().map_or(false, |key| key == "Dirty:") {
            return if let Some(value) = iter.next() {
                match value.parse::<u64>() {
                    // multiply by 2 for KB to # of sectors conversion
                    Ok(dirty_mem_size) => Ok(Sectors(dirty_mem_size * 2)),
                    Err(_) => Err(StratisError::Engine(
                        ErrorEnum::Invalid,
                        format!(
                            "Failed to parse value for \"Dirty\" key from /proc/meminfo : {:?}",
                            value
                        ),
                    )),
                }
            } else {
                Err(StratisError::Engine(
                    ErrorEnum::Invalid,
                    "No value found for \"Dirty\" key in /proc/meminfo".into(),
                ))
            };
        }
    }
    Err(StratisError::Engine(
        ErrorEnum::Invalid,
        "\"Dirty\" key not found in /prop/meminfo".into(),
    ))
}

/// Transform a list of segments belonging to a single device into a
/// list of target lines for a linear device.
fn segs_to_table(
    dev: Device,
    segments: &[(Sectors, Sectors)],
) -> Vec<TargetLine<LinearDevTargetParams>> {
    let mut table = Vec::new();
    let mut logical_start_offset = Sectors(0);

    for &(start_offset, length) in segments {
        let params = LinearTargetParams::new(dev, start_offset);
        let line = TargetLine::new(
            logical_start_offset,
            length,
            LinearDevTargetParams::Linear(params),
        );
        table.push(line);
        logical_start_offset += length;
    }
    table
}

/// Append the second list of segments to the first, or if the last
/// segment of the first argument is adjacent to the first segment of the
/// second argument, merge those two together.
/// Postcondition: left.len() + right.len() - 1 <= result.len()
/// Postcondition: result.len() <= left.len() + right.len()
// FIXME: There is a method that duplicates this algorithm called
// coalesce_blkdevsegs. These methods should either be unified into a single
// method OR one should go away entirely in solution to:
// https://github.com/stratis-storage/stratisd/issues/762.
fn coalesce_segs(
    left: &[(Sectors, Sectors)],
    right: &[(Sectors, Sectors)],
) -> Vec<(Sectors, Sectors)> {
    if left.is_empty() {
        return right.to_vec();
    }
    if right.is_empty() {
        return left.to_vec();
    }

    let mut segments = Vec::with_capacity(left.len() + right.len());
    segments.extend_from_slice(left);

    // Combine first and last if they are contiguous.
    let coalesced = {
        let right_first = right.first().expect("!right.is_empty()");
        let left_last = segments.last_mut().expect("!left.is_empty()");
        if left_last.0 + left_last.1 == right_first.0 {
            left_last.1 += right_first.1;
            true
        } else {
            false
        }
    };

    if coalesced {
        segments.extend_from_slice(&right[1..]);
    } else {
        segments.extend_from_slice(right);
    }
    segments
}

/// Calculate new low water based on the current thinpool data device size and
/// the number of free sectors in the backstore (free in data tier; or
/// allocated *to* the backstore cap device, but not yet allocated *from* the
/// cap device.)
/// Postcondition:
/// result == max(M * (data_dev_size + available) - available, L)
/// equivalently:
/// result == max(M * data_dev_size - (1 - M) * available, L)
/// where M <= (100 - SPACE_WARN_PCT)/100 if self.free_space_state == Good
///            (100 - SPACE_CRIT_PCT)/100  if self.free_space_state != Good
///       L = DATA_LOWATER if self.free_space_state == Good
///           throttle rate if self.free_space_state != Good
// TODO: Use proptest to verify the behavior of this method.
fn calc_lowater(
    data_dev_size: DataBlocks,
    available: DataBlocks,
    free_space_state: FreeSpaceState,
) -> DataBlocks {
    // Calculate the low water. dev_low_water and action_pct are the device
    // low water and the percent used at which an action should be taken for
    // a particular free space state.
    //
    // Postcondition:
    // result == max(M * data_dev_size - (1 - M) * available, dev_low_water)
    // where M <= (100 - action_pct)/100
    let calc_lowater_internal = |dev_low_water: DataBlocks, action_pct: u8| -> DataBlocks {
        let total = data_dev_size + available;

        assert!(action_pct <= 100);
        let low_water = total - ((total * action_pct) / 100u8);
        assert!(DataBlocks(std::u64::MAX) - available >= dev_low_water);

        // WARNING: Do not alter this if-expression to a max-expression.
        // Doing so would invalidate the assertion below.
        if dev_low_water + available > low_water {
            dev_low_water
        } else {
            assert!(low_water >= available);
            low_water - available
        }
    };

    match free_space_state {
        FreeSpaceState::Good => calc_lowater_internal(DATA_LOWATER, SPACE_WARN_PCT),
        _ => calc_lowater_internal(THROTTLE_BLOCKS_PER_SEC, SPACE_CRIT_PCT),
    }
}

/// Segment lists that the ThinPool keeps track of.
#[derive(Debug)]
struct Segments {
    meta_segments: Vec<(Sectors, Sectors)>,
    meta_spare_segments: Vec<(Sectors, Sectors)>,
    data_segments: Vec<(Sectors, Sectors)>,
    mdv_segments: Vec<(Sectors, Sectors)>,
}

pub struct ThinPoolSizeParams {
    meta_size: MetaBlocks,
    data_size: DataBlocks,
    mdv_size: Sectors,
}

impl ThinPoolSizeParams {
    /// The number of Sectors in the MetaBlocks.
    pub fn meta_size(&self) -> Sectors {
        self.meta_size.sectors()
    }
    /// The number of Sectors in the DataBlocks.
    pub fn data_size(&self) -> Sectors {
        datablocks_to_sectors(self.data_size)
    }
    /// MDV size
    pub fn mdv_size(&self) -> Sectors {
        self.mdv_size
    }
}

impl Default for ThinPoolSizeParams {
    fn default() -> ThinPoolSizeParams {
        ThinPoolSizeParams {
            meta_size: INITIAL_META_SIZE,
            data_size: INITIAL_DATA_SIZE,
            mdv_size: INITIAL_MDV_SIZE,
        }
    }
}

/// A ThinPool struct contains the thinpool itself, the spare
/// segments for its metadata device, and the filesystems and filesystem
/// metadata associated with it.
#[derive(Debug)]
pub struct ThinPool {
    thin_pool: ThinPoolDev,
    segments: Segments,
    id_gen: ThinDevIdPool,
    filesystems: Table<StratFilesystem>,
    mdv: MetadataVol,
    /// The single DM device that the backstore presents as its upper-most
    /// layer. All DM components obtain their storage from this layer.
    /// The device will change if the backstore adds or removes a cache.
    backstore_device: Device,
    pool_state: PoolState,
    pool_extend_state: PoolExtendState,
    free_space_state: FreeSpaceState,
    dbus_path: MaybeDbusPath,
}

impl ThinPool {
    /// Make a new thin pool.
    pub fn new(
        pool_uuid: PoolUuid,
        thin_pool_size: &ThinPoolSizeParams,
        data_block_size: Sectors,
        backstore: &mut Backstore,
    ) -> StratisResult<ThinPool> {
        let mut segments_list = match backstore.alloc(
            pool_uuid,
            &[
                thin_pool_size.meta_size(),
                thin_pool_size.meta_size(),
                thin_pool_size.data_size(),
                thin_pool_size.mdv_size(),
            ],
        )? {
            Some(sl) => sl,
            None => {
                let err_msg = "Could not allocate sufficient space for thinpool devices.";
                return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg.into()));
            }
        };

        let mdv_segments = segments_list.pop().expect("len(segments_list) == 4");
        let data_segments = segments_list.pop().expect("len(segments_list) == 3");
        let spare_segments = segments_list.pop().expect("len(segments_list) == 2");
        let meta_segments = segments_list.pop().expect("len(segments_list) == 1");

        let backstore_device = backstore.device().expect(
            "Space has just been allocated from the backstore, so it must have a cap device",
        );

        // When constructing a thin-pool, Stratis reserves the first N
        // sectors on a block device by creating a linear device with a
        // starting offset. DM writes the super block in the first block.
        // DM requires this first block to be zeros when the meta data for
        // the thin-pool is initially created. If we don't zero the
        // superblock DM issue error messages because it triggers code paths
        // that are trying to re-adopt the device with the attributes that
        // have been passed.
        let (dm_name, dm_uuid) = format_flex_ids(pool_uuid, FlexRole::ThinMeta);
        let meta_dev = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            segs_to_table(backstore_device, &[meta_segments]),
        )?;

        // Wipe the first 4 KiB, i.e. 8 sectors as recommended in kernel DM
        // docs: device-mapper/thin-provisioning.txt: Setting up a fresh
        // pool device.
        wipe_sectors(
            &meta_dev.devnode(),
            Sectors(0),
            min(Sectors(8), meta_dev.size()),
        )?;

        let (dm_name, dm_uuid) = format_flex_ids(pool_uuid, FlexRole::ThinData);
        let data_dev = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            segs_to_table(backstore_device, &[data_segments]),
        )?;

        let (dm_name, dm_uuid) = format_flex_ids(pool_uuid, FlexRole::MetadataVolume);
        let mdv_dev = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            segs_to_table(backstore_device, &[mdv_segments]),
        )?;
        let mdv = MetadataVol::initialize(pool_uuid, mdv_dev)?;

        let (dm_name, dm_uuid) = format_thinpool_ids(pool_uuid, ThinPoolRole::Pool);

        let (free_space_state, data_dev_size) = (FreeSpaceState::Good, data_dev.size());
        let thinpool_dev = ThinPoolDev::new(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            meta_dev,
            data_dev,
            data_block_size,
            calc_lowater(
                sectors_to_datablocks(data_dev_size),
                sectors_to_datablocks(backstore.available_in_backstore()),
                free_space_state,
            ),
        )?;

        Ok(ThinPool {
            thin_pool: thinpool_dev,
            segments: Segments {
                meta_segments: vec![meta_segments],
                meta_spare_segments: vec![spare_segments],
                data_segments: vec![data_segments],
                mdv_segments: vec![mdv_segments],
            },
            id_gen: ThinDevIdPool::new_from_ids(&[]),
            filesystems: Table::default(),
            mdv,
            backstore_device,
            pool_state: PoolState::Initializing,
            pool_extend_state: PoolExtendState::Initializing,
            free_space_state,
            dbus_path: MaybeDbusPath(None),
        })
    }

    /// Set up an "existing" thin pool.
    /// A thin pool must store the metadata for its thin devices, regardless of
    /// whether it has an existing device node. An existing thin pool device
    /// is a device where the metadata is already stored on its meta device.
    /// If initial setup fails due to a thin_check failure, attempt to fix
    /// the problem by running thin_repair. If failure recurs, return an
    /// error.
    pub fn setup(
        pool_uuid: PoolUuid,
        thin_pool_save: &ThinPoolDevSave,
        flex_devs: &FlexDevsSave,
        backstore: &Backstore,
    ) -> StratisResult<ThinPool> {
        let mdv_segments = flex_devs.meta_dev.to_vec();
        let meta_segments = flex_devs.thin_meta_dev.to_vec();
        let data_segments = flex_devs.thin_data_dev.to_vec();
        let spare_segments = flex_devs.thin_meta_dev_spare.to_vec();

        let backstore_device = backstore.device().expect("When stratisd was running previously, space was allocated from the backstore, so backstore must have a cap device");

        let (thinpool_name, thinpool_uuid) = format_thinpool_ids(pool_uuid, ThinPoolRole::Pool);
        let (meta_dev, meta_segments, spare_segments) = setup_metadev(
            pool_uuid,
            &thinpool_name,
            backstore_device,
            meta_segments,
            spare_segments,
        )?;

        let (dm_name, dm_uuid) = format_flex_ids(pool_uuid, FlexRole::ThinData);
        let data_dev = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            segs_to_table(backstore_device, &data_segments),
        )?;

        let (free_space_state, data_dev_size) = (FreeSpaceState::Good, data_dev.size());
        let thinpool_dev = ThinPoolDev::setup(
            get_dm(),
            &thinpool_name,
            Some(&thinpool_uuid),
            meta_dev,
            data_dev,
            thin_pool_save.data_block_size,
            calc_lowater(
                sectors_to_datablocks(data_dev_size),
                sectors_to_datablocks(backstore.available_in_backstore()),
                free_space_state,
            ),
        )?;

        let (dm_name, dm_uuid) = format_flex_ids(pool_uuid, FlexRole::MetadataVolume);
        let mdv_dev = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            segs_to_table(backstore_device, &mdv_segments),
        )?;
        let mdv = MetadataVol::setup(pool_uuid, mdv_dev)?;
        let filesystem_metadatas = mdv.filesystems()?;

        let filesystems = filesystem_metadatas
            .iter()
            .filter_map(
                |fssave| match StratFilesystem::setup(pool_uuid, &thinpool_dev, fssave) {
                    Ok(fs) => Some((Name::new(fssave.name.to_owned()), fssave.uuid, fs)),
                    Err(err) => {
                        warn!(
                            "Filesystem specified by metadata {:?} could not be setup, reason: {:?}",
                            fssave,
                            err
                        );
                        None
                    }
                },
            )
            .collect::<Vec<_>>();

        let mut fs_table = Table::default();
        for (name, uuid, fs) in filesystems {
            let evicted = fs_table.insert(name, uuid, fs);
            if evicted.is_some() {
                // TODO: Recover here. Failing the entire pool setup because
                // of this is too harsh.
                let err_msg = "filesystems with duplicate UUID or name specified in metadata";
                return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg.into()));
            }
        }

        let thin_ids: Vec<ThinDevId> = filesystem_metadatas.iter().map(|x| x.thin_id).collect();
        Ok(ThinPool {
            thin_pool: thinpool_dev,
            segments: Segments {
                meta_segments,
                meta_spare_segments: spare_segments,
                data_segments,
                mdv_segments,
            },
            id_gen: ThinDevIdPool::new_from_ids(&thin_ids),
            filesystems: fs_table,
            mdv,
            backstore_device,
            pool_state: PoolState::Initializing,
            pool_extend_state: PoolExtendState::Initializing,
            free_space_state,
            dbus_path: MaybeDbusPath(None),
        })
    }

    /// Run status checks and take actions on the thinpool and its components.
    /// Returns a bool communicating if a configuration change requiring a
    /// metadata save has been made.
    pub fn check(&mut self, pool_uuid: PoolUuid, backstore: &mut Backstore) -> StratisResult<bool> {
        let mut should_save = false;

        // Re-run to ensure pool status is updated if we made any changes
        while self.do_check(pool_uuid, backstore)? {
            should_save = true;
        }

        Ok(should_save)
    }

    /// Do the real work of check().
    fn do_check(&mut self, pool_uuid: PoolUuid, backstore: &mut Backstore) -> StratisResult<bool> {
        // Calculate amount to request for data- or meta- device.
        // Return None if device does not need to be expanded.
        // Returned request, if it exists, is always INITIAL_META_SIZE
        // for meta device, 1 Gi for data device.
        // Since one event can have many potential causes (meta extension,
        // data extension OR space state check), check remaining against
        // low_water to see if our condition was even the cause of the event.
        fn calculate_extension_request(
            total: Sectors,
            used: Sectors,
            low_water: Sectors,
            data: bool,
        ) -> Option<Sectors> {
            let remaining = total - used;
            if remaining <= low_water {
                Some(if data {
                    match current_dirty_mem() {
                        Ok(dirty_mem_size) => max(dirty_mem_size, DATA_EXPAND_SIZE),
                        Err(_) => DATA_EXPAND_SIZE,
                    }
                } else {
                    INITIAL_META_SIZE.sectors()
                })
            } else {
                None
            }
        }

        assert_eq!(
            backstore.device().expect(
                "thinpool exists and has been allocated to, so backstore must have a cap device"
            ),
            self.backstore_device
        );

        let mut should_save: bool = false;
        match self.thin_pool.status(get_dm())? {
            ThinPoolStatus::Working(ref status) => {
                let mut meta_extend_failed = false;
                let mut data_extend_failed = false;
                match status.summary {
                    ThinPoolStatusSummary::Good => {
                        self.set_state(PoolState::Running);
                    }
                    // If a pool is in ReadOnly mode it is due to either meta data full or
                    // the pool requires repair.
                    ThinPoolStatusSummary::ReadOnly => {
                        error!("Thinpool read only! -> ReadOnly");
                        self.set_state(PoolState::ReadOnly);
                    }
                    ThinPoolStatusSummary::OutOfSpace => {
                        error!("Thinpool out of space! -> OutOfSpace");
                        self.set_state(PoolState::OutOfDataSpace);
                    }
                }

                let usage = &status.usage;

                // Kernel 4.19+ includes the kernel-set meta lowater value in
                // thinpool status. For older kernels, use a default value.
                let meta_lowater = status
                    .meta_low_water
                    .map(MetaBlocks)
                    .unwrap_or(META_LOWATER_FALLBACK);
                if let Some(request) = calculate_extension_request(
                    usage.total_meta.sectors(),
                    usage.used_meta.sectors(),
                    meta_lowater.sectors(),
                    false,
                ) {
                    let meta_extend_failed =
                        match self.extend_thin_meta_device(pool_uuid, backstore, request) {
                            Ok(extend_size) => extend_size == Sectors(0),
                            Err(_) => true,
                        };
                    should_save |= !meta_extend_failed;
                }

                let extend_size = sectors_to_datablocks({
                    match calculate_extension_request(
                        datablocks_to_sectors(usage.total_data),
                        datablocks_to_sectors(usage.used_data),
                        datablocks_to_sectors(self.thin_pool.table().table.params.low_water_mark),
                        true,
                    ) {
                        None => Sectors(0),
                        Some(request) => {
                            let amount_allocated =
                                match self.extend_thin_data_device(pool_uuid, backstore, request) {
                                    Ok(extend_size) => extend_size,
                                    Err(_) => Sectors(0),
                                };
                            data_extend_failed = amount_allocated == Sectors(0);
                            should_save |= !data_extend_failed;
                            amount_allocated
                        }
                    }
                });

                let current_total = usage.total_data + extend_size;

                // Update pool space state
                self.free_space_check(
                    usage.used_data,
                    current_total + sectors_to_datablocks(backstore.available_in_backstore())
                        - usage.used_data,
                )?;

                // Trigger next event depending on pool space state
                let lowater = calc_lowater(
                    current_total,
                    sectors_to_datablocks(backstore.available_in_backstore()),
                    self.free_space_state,
                );

                self.thin_pool.set_low_water_mark(get_dm(), lowater)?;
                self.resume()?;
                self.set_extend_state(data_extend_failed, meta_extend_failed);
            }
            ThinPoolStatus::Fail => {
                error!("Thinpool status is fail -> Failed");
                self.set_state(PoolState::Failed);
                // TODO: Take pool offline?
                // TODO: Run thin_check
            }
        }

        let filesystems = self.filesystems
            .borrow_mut()
            .iter_mut()
            .map(|(_, _, fs)| fs.check())
            .collect::<StratisResult<Vec<_>>>()?;

        for fs_status in filesystems {
            if let FilesystemStatus::Failed = fs_status {
                // TODO: filesystem failed, how to recover?
            }
        }
        Ok(should_save)
    }

    fn set_state(&mut self, new_state: PoolState) {
        if self.state() != new_state {
            self.pool_state = new_state;
            get_engine_listener_list().notify(&EngineEvent::PoolStateChanged {
                dbus_path: self.get_dbus_path(),
                state: new_state,
            });
        }
    }

    fn set_extend_state(&mut self, data_extend_failed: bool, meta_extend_failed: bool) {
        let mut new_state = PoolExtendState::Good;
        if data_extend_failed && meta_extend_failed {
            new_state = PoolExtendState::MetaAndDataFailed
        } else if data_extend_failed {
            new_state = PoolExtendState::DataFailed;
        } else if meta_extend_failed {
            new_state = PoolExtendState::MetaFailed;
        }
        if self.extend_state() != new_state {
            self.pool_extend_state = new_state;
            get_engine_listener_list().notify(&EngineEvent::PoolExtendStateChanged {
                dbus_path: self.get_dbus_path(),
                state: new_state,
            });
        }
    }

    fn set_free_space_state(&mut self, new_state: FreeSpaceState) {
        if self.free_space_state() != new_state {
            self.free_space_state = new_state;
            get_engine_listener_list().notify(&EngineEvent::PoolSpaceStateChanged {
                dbus_path: self.get_dbus_path(),
                state: new_state,
            });
        }
    }

    /// Possibly transition to a new FreeSpaceState based on usage, and invoke
    /// policies (throttling, suspension) accordingly.
    fn free_space_check(
        &mut self,
        used: DataBlocks,
        available: DataBlocks,
    ) -> StratisResult<FreeSpaceState> {
        // Return a value from 0 to 100 that is the percentage that "used"
        // makes up in "total".
        fn used_pct(used: u64, total: u64) -> u8 {
            assert!(total >= used);
            let mut val = (used * 100) / total;
            if (used * 100) % total != 0 {
                val += 1; // round up
            }
            assert!(val <= 100);
            val as u8
        }

        let overall_used_pct = used_pct(*used, *used + *available);
        info!("Data tier percent used: {}", overall_used_pct);

        let new_state = if overall_used_pct < SPACE_WARN_PCT {
            FreeSpaceState::Good
        } else if overall_used_pct < SPACE_CRIT_PCT {
            FreeSpaceState::Warn
        } else {
            FreeSpaceState::Crit
        };

        self.set_free_space_state(new_state);

        match (self.free_space_state, new_state) {
            (FreeSpaceState::Good, FreeSpaceState::Warn) => {
                // TODO: other steps to regain space: schedule fstrims?
                set_write_throttling(
                    self.thin_pool.data_dev().device(),
                    Some(datablocks_to_sectors(THROTTLE_BLOCKS_PER_SEC).bytes()),
                )?;
            }
            (FreeSpaceState::Good, FreeSpaceState::Crit) => {
                set_write_throttling(
                    self.thin_pool.data_dev().device(),
                    Some(datablocks_to_sectors(THROTTLE_BLOCKS_PER_SEC).bytes()),
                )?;

                for (_, _, fs) in &mut self.filesystems {
                    fs.suspend(true)?;
                }
            }
            (FreeSpaceState::Warn, FreeSpaceState::Good) => {
                set_write_throttling(self.thin_pool.data_dev().device(), None)?;
            }
            (FreeSpaceState::Warn, FreeSpaceState::Crit) => {
                for (_, _, fs) in &mut self.filesystems {
                    fs.suspend(true)?;
                }
            }
            (FreeSpaceState::Crit, FreeSpaceState::Good) => {
                for (_, _, fs) in &mut self.filesystems {
                    fs.resume()?;
                }
                set_write_throttling(self.thin_pool.data_dev().device(), None)?;
            }
            (FreeSpaceState::Crit, FreeSpaceState::Warn) => {
                for (_, _, fs) in &mut self.filesystems {
                    fs.resume()?;
                }
            }
            // These all represent no change in the state, so nothing is done.
            (old, new) => assert_eq!(old, new),
        };

        Ok(new_state)
    }

    /// Tear down the components managed here: filesystems, the MDV,
    /// and the actual thinpool device itself.
    pub fn teardown(&mut self) -> StratisResult<()> {
        self.set_state(PoolState::Stopping);
        // Must succeed in tearing down all filesystems before the
        // thinpool..
        for (_, _, ref mut fs) in &mut self.filesystems {
            fs.teardown()?;
        }
        self.thin_pool.teardown(get_dm())?;

        // ..but MDV has no DM dependencies with the above
        self.mdv.teardown()?;

        Ok(())
    }

    /// Extend thinpool's data dev. See extend_thin_sub_device for more info.
    fn extend_thin_data_device(
        &mut self,
        pool_uuid: PoolUuid,
        backstore: &mut Backstore,
        extend_size: Sectors,
    ) -> StratisResult<Sectors> {
        ThinPool::extend_thin_sub_device(
            pool_uuid,
            &mut self.thin_pool,
            backstore,
            extend_size,
            DATA_BLOCK_SIZE,
            &mut self.segments.data_segments,
            true,
        )
    }

    /// Extend thinpool's meta dev. See extend_thin_sub_device for more info.
    fn extend_thin_meta_device(
        &mut self,
        pool_uuid: PoolUuid,
        backstore: &mut Backstore,
        extend_size: Sectors,
    ) -> StratisResult<Sectors> {
        ThinPool::extend_thin_sub_device(
            pool_uuid,
            &mut self.thin_pool,
            backstore,
            extend_size,
            MetaBlocks(1).sectors(),
            &mut self.segments.meta_segments,
            false,
        )
    }

    /// Extend the thinpool's data or meta devices. The result is the value
    /// by which the device is extended which may be less than the requested
    /// amount. It is guaranteed that the returned amount is a multiple of the
    /// modulus value. The amount returned may be 0, if nothing could be
    /// allocated. Sets existing_segs to the new value that specifies the
    /// arrangement of segments on the extended device. The data parameter is
    /// true if the method should extend the data device, false if the
    /// method should extend the meta device.
    fn extend_thin_sub_device(
        pool_uuid: PoolUuid,
        thinpooldev: &mut ThinPoolDev,
        backstore: &mut Backstore,
        extend_size: Sectors,
        modulus: Sectors,
        existing_segs: &mut Vec<(Sectors, Sectors)>,
        data: bool,
    ) -> StratisResult<Sectors> {
        let sub_device_str = if data { "data" } else { "metadata" };
        let pool_uuid_str = pool_uuid.simple().to_string();
        info!(
            "Attempting to extend thinpool {} sub-device belonging to pool {} by {}",
            sub_device_str, pool_uuid_str, extend_size,
        );

        let result = if let Some(region) = backstore.request(pool_uuid, extend_size, modulus)? {
            let device = backstore
                .device()
                .expect("If request succeeded, backstore must have cap device.");
            let mut segments = coalesce_segs(existing_segs, &[region]);
            if data {
                thinpooldev.set_data_table(get_dm(), segs_to_table(device, &segments))?;
            } else {
                thinpooldev.set_meta_table(get_dm(), segs_to_table(device, &segments))?;
            }

            thinpooldev.resume(get_dm())?;
            existing_segs.clear();
            existing_segs.append(&mut segments);

            Ok(region.1)
        } else {
            Ok(Sectors(0))
        };
        match result {
            Ok(actual_extend_size) => {
                if actual_extend_size == extend_size {
                    info!(
                        "Extended thinpool {} sub-device belonging to pool with uuid {} by {}",
                        sub_device_str, pool_uuid_str, actual_extend_size
                    );
                } else {
                    warn!("Insufficient free space available in backstore; extended thinpool {} sub-device belonging to pool with uuid {} by {}, request was {}",
                      sub_device_str,
                      pool_uuid_str,
                      actual_extend_size,
                      extend_size);
                }
            }
            Err(ref err) => {
                error!("Attempted to extend thinpool {} sub-device belonging to pool with uuid {} by {} but failed with error: {:?}",
                       sub_device_str,
                       pool_uuid_str,
                       extend_size,
                       err);
            }
        }
        result
    }

    /// The number of physical sectors in use, that is, unavailable for storage
    /// of additional user data, by this pool.
    // This includes all the sectors being held as spares for the meta device,
    // all the sectors allocated to the meta data device, and all the sectors
    // in use on the data device.
    pub fn total_physical_used(&self) -> StratisResult<Sectors> {
        let data_dev_used = match self.thin_pool.status(get_dm())? {
            ThinPoolStatus::Working(ref status) => datablocks_to_sectors(status.usage.used_data),
            _ => {
                let err_msg = "thin pool failed, could not obtain usage";
                return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg.into()));
            }
        };

        let spare_total = self.segments.meta_spare_segments.iter().map(|s| s.1).sum();

        let meta_dev_total = self.thin_pool.meta_dev().size();

        let mdv_total = self.segments.mdv_segments.iter().map(|s| s.1).sum();

        Ok(data_dev_used + spare_total + meta_dev_total + mdv_total)
    }

    pub fn get_filesystem_by_uuid(&self, uuid: FilesystemUuid) -> Option<(Name, &StratFilesystem)> {
        self.filesystems.get_by_uuid(uuid)
    }

    pub fn get_mut_filesystem_by_uuid(
        &mut self,
        uuid: FilesystemUuid,
    ) -> Option<(Name, &mut StratFilesystem)> {
        self.filesystems.get_mut_by_uuid(uuid)
    }

    pub fn get_filesystem_by_name(&self, name: &str) -> Option<(FilesystemUuid, &StratFilesystem)> {
        self.filesystems.get_by_name(name)
    }

    pub fn get_mut_filesystem_by_name(
        &mut self,
        name: &str,
    ) -> Option<(FilesystemUuid, &mut StratFilesystem)> {
        self.filesystems.get_mut_by_name(name)
    }

    pub fn has_filesystems(&self) -> bool {
        !self.filesystems.is_empty()
    }

    pub fn filesystems(&self) -> Vec<(Name, FilesystemUuid, &Filesystem)> {
        self.filesystems
            .iter()
            .map(|(name, uuid, x)| (name.clone(), *uuid, x as &Filesystem))
            .collect()
    }

    pub fn filesystems_mut(&mut self) -> Vec<(Name, FilesystemUuid, &mut Filesystem)> {
        self.filesystems
            .iter_mut()
            .map(|(name, uuid, x)| (name.clone(), *uuid, x as &mut Filesystem))
            .collect()
    }

    /// Create a filesystem within the thin pool. Given name must not
    /// already be in use.
    pub fn create_filesystem(
        &mut self,
        pool_uuid: PoolUuid,
        pool_name: &str,
        name: &str,
        size: Option<Sectors>,
    ) -> StratisResult<FilesystemUuid> {
        let (fs_uuid, mut new_filesystem) =
            StratFilesystem::initialize(pool_uuid, &self.thin_pool, size, self.id_gen.new_id()?)?;
        let name = Name::new(name.to_owned());
        if let Err(err) = self.mdv.save_fs(&name, fs_uuid, &new_filesystem) {
            fs_settle();
            if let Err(err2) = new_filesystem.destroy(&self.thin_pool) {
                error!(
                    "When handling failed save_fs(), fs.destroy() failed: {}",
                    err2
                )
            }
            return Err(err);
        }
        devlinks::filesystem_added(pool_name, &name, &new_filesystem.devnode());
        self.filesystems.insert(name, fs_uuid, new_filesystem);

        Ok(fs_uuid)
    }

    /// Create a filesystem snapshot of the origin.  Given origin_uuid
    /// must exist.  Returns the Uuid of the new filesystem.
    pub fn snapshot_filesystem(
        &mut self,
        pool_uuid: PoolUuid,
        pool_name: &str,
        origin_uuid: FilesystemUuid,
        snapshot_name: &str,
    ) -> StratisResult<(FilesystemUuid, &mut Filesystem)> {
        let snapshot_fs_uuid = Uuid::new_v4();
        let (snapshot_dm_name, snapshot_dm_uuid) =
            format_thin_ids(pool_uuid, ThinRole::Filesystem(snapshot_fs_uuid));
        let snapshot_id = self.id_gen.new_id()?;
        let new_filesystem = match self.get_filesystem_by_uuid(origin_uuid) {
            Some((fs_name, filesystem)) => filesystem.snapshot(
                &self.thin_pool,
                snapshot_name,
                &snapshot_dm_name,
                Some(&snapshot_dm_uuid),
                &fs_name,
                snapshot_fs_uuid,
                snapshot_id,
            )?,
            None => {
                return Err(StratisError::Engine(
                    ErrorEnum::Error,
                    "snapshot_filesystem failed, filesystem not found".into(),
                ));
            }
        };
        let new_fs_name = Name::new(snapshot_name.to_owned());
        self.mdv
            .save_fs(&new_fs_name, snapshot_fs_uuid, &new_filesystem)?;
        devlinks::filesystem_added(pool_name, &new_fs_name, &new_filesystem.devnode());
        self.filesystems
            .insert(new_fs_name, snapshot_fs_uuid, new_filesystem);
        Ok((
            snapshot_fs_uuid,
            self.filesystems
                .get_mut_by_uuid(snapshot_fs_uuid)
                .expect("just inserted")
                .1,
        ))
    }

    /// Destroy a filesystem within the thin pool. Destroy metadata and
    /// devlinks information associated with the thinpool. If there is a
    /// failure to destroy the filesystem, retain it, and return an error.
    pub fn destroy_filesystem(
        &mut self,
        pool_name: &str,
        uuid: FilesystemUuid,
    ) -> StratisResult<()> {
        match self.filesystems.remove_by_uuid(uuid) {
            Some((fs_name, mut fs)) => match fs.destroy(&self.thin_pool) {
                Ok(_) => {
                    if let Err(err) = self.mdv.rm_fs(uuid) {
                        error!("Could not remove metadata for fs with UUID {} and name {} belonging to pool {}, reason: {:?}",
                               uuid,
                               fs_name,
                               pool_name,
                               err);
                    }
                    devlinks::filesystem_removed(pool_name, &fs_name);
                    Ok(())
                }
                Err(err) => {
                    self.filesystems.insert(fs_name, uuid, fs);
                    Err(err)
                }
            },
            None => Ok(()),
        }
    }

    pub fn state(&self) -> PoolState {
        self.pool_state
    }

    pub fn extend_state(&self) -> PoolExtendState {
        self.pool_extend_state
    }

    pub fn free_space_state(&self) -> FreeSpaceState {
        self.free_space_state
    }

    /// Rename a filesystem within the thin pool.
    pub fn rename_filesystem(
        &mut self,
        pool_name: &str,
        uuid: FilesystemUuid,
        new_name: &str,
    ) -> StratisResult<RenameAction> {
        let old_name = rename_filesystem_pre!(self; uuid; new_name);
        let new_name = Name::new(new_name.to_owned());

        let filesystem = self.filesystems
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.filesystems.get_by_uuid() returned a value")
            .1;

        if let Err(err) = self.mdv.save_fs(&new_name, uuid, &filesystem) {
            self.filesystems.insert(old_name, uuid, filesystem);
            Err(err)
        } else {
            get_engine_listener_list().notify(&EngineEvent::FilesystemRenamed {
                dbus_path: filesystem.get_dbus_path(),
                from: &*old_name,
                to: &*new_name,
            });
            self.filesystems.insert(new_name.clone(), uuid, filesystem);
            devlinks::filesystem_renamed(pool_name, &old_name, &new_name);
            Ok(RenameAction::Renamed)
        }
    }

    /// The names of DM devices belonging to this pool that may generate events
    pub fn get_eventing_dev_names(&self, pool_uuid: PoolUuid) -> Vec<DmNameBuf> {
        vec![
            format_flex_ids(pool_uuid, FlexRole::ThinMeta).0,
            format_flex_ids(pool_uuid, FlexRole::ThinData).0,
            format_flex_ids(pool_uuid, FlexRole::MetadataVolume).0,
            format_thinpool_ids(pool_uuid, ThinPoolRole::Pool).0,
        ]
    }

    /// Suspend the thinpool
    pub fn suspend(&mut self) -> StratisResult<()> {
        // thindevs automatically suspended when thinpool is suspended
        self.thin_pool.suspend(get_dm(), true)?;
        self.mdv.suspend()?;
        Ok(())
    }

    /// Resume the thinpool
    pub fn resume(&mut self) -> StratisResult<()> {
        self.mdv.resume()?;
        // thindevs automatically resumed here
        self.thin_pool.resume(get_dm())?;
        Ok(())
    }

    /// Set the device on all DM devices
    pub fn set_device(&mut self, backstore_device: Device) -> StratisResult<bool> {
        if backstore_device == self.backstore_device {
            return Ok(false);
        }

        let xform_target_line =
            |line: &TargetLine<LinearDevTargetParams>| -> TargetLine<LinearDevTargetParams> {
                let new_params = match line.params {
                    LinearDevTargetParams::Linear(ref params) => LinearDevTargetParams::Linear(
                        LinearTargetParams::new(backstore_device, params.start_offset),
                    ),
                    LinearDevTargetParams::Flakey(ref params) => {
                        let feature_args = params.feature_args.iter().cloned().collect::<Vec<_>>();
                        LinearDevTargetParams::Flakey(FlakeyTargetParams::new(
                            backstore_device,
                            params.start_offset,
                            params.up_interval,
                            params.down_interval,
                            feature_args,
                        ))
                    }
                };

                TargetLine::new(line.start, line.length, new_params)
            };

        let meta_table = self.thin_pool
            .meta_dev()
            .table()
            .table
            .clone()
            .iter()
            .map(&xform_target_line)
            .collect::<Vec<_>>();

        let data_table = self.thin_pool
            .data_dev()
            .table()
            .table
            .clone()
            .iter()
            .map(&xform_target_line)
            .collect::<Vec<_>>();

        let mdv_table = self.mdv
            .device()
            .table()
            .table
            .clone()
            .iter()
            .map(&xform_target_line)
            .collect::<Vec<_>>();

        self.thin_pool.set_meta_table(get_dm(), meta_table)?;
        self.thin_pool.set_data_table(get_dm(), data_table)?;
        self.mdv.set_table(mdv_table)?;

        self.backstore_device = backstore_device;

        Ok(true)
    }

    pub fn set_dbus_path(&mut self, path: MaybeDbusPath) -> () {
        self.dbus_path = path
    }

    fn get_dbus_path(&self) -> &MaybeDbusPath {
        &self.dbus_path
    }
}

impl Recordable<FlexDevsSave> for Segments {
    fn record(&self) -> FlexDevsSave {
        FlexDevsSave {
            meta_dev: self.mdv_segments.to_vec(),
            thin_meta_dev: self.meta_segments.to_vec(),
            thin_data_dev: self.data_segments.to_vec(),
            thin_meta_dev_spare: self.meta_spare_segments.to_vec(),
        }
    }
}

impl Recordable<FlexDevsSave> for ThinPool {
    fn record(&self) -> FlexDevsSave {
        self.segments.record()
    }
}

impl Recordable<ThinPoolDevSave> for ThinPool {
    fn record(&self) -> ThinPoolDevSave {
        ThinPoolDevSave {
            data_block_size: self.thin_pool.data_block_size(),
        }
    }
}

/// Setup metadata dev for thinpool.
/// Attempt to verify that the metadata dev is valid for the given thinpool
/// using thin_check. If thin_check indicates that the metadata is corrupted
/// run thin_repair, using the spare segments, to try to repair the metadata
/// dev. Return the metadata device, the metadata segments, and the
/// spare segments.
#[allow(type_complexity)]
fn setup_metadev(
    pool_uuid: PoolUuid,
    thinpool_name: &DmName,
    device: Device,
    meta_segments: Vec<(Sectors, Sectors)>,
    spare_segments: Vec<(Sectors, Sectors)>,
) -> StratisResult<(LinearDev, Vec<(Sectors, Sectors)>, Vec<(Sectors, Sectors)>)> {
    #![allow(collapsible_if)]
    let (dm_name, dm_uuid) = format_flex_ids(pool_uuid, FlexRole::ThinMeta);
    let mut meta_dev = LinearDev::setup(
        get_dm(),
        &dm_name,
        Some(&dm_uuid),
        segs_to_table(device, &meta_segments),
    )?;

    if !device_exists(get_dm(), thinpool_name)? {
        // TODO: Refine policy about failure to run thin_check.
        // If, e.g., thin_check is unavailable, that doesn't necessarily
        // mean that data is corrupted.
        if thin_check(&meta_dev.devnode()).is_err() {
            meta_dev = attempt_thin_repair(pool_uuid, meta_dev, device, &spare_segments)?;
            return Ok((meta_dev, spare_segments, meta_segments));
        }
    }

    Ok((meta_dev, meta_segments, spare_segments))
}

/// Attempt a thin repair operation on the meta device.
/// If the operation succeeds, teardown the old meta device,
/// and return the new meta device.
fn attempt_thin_repair(
    pool_uuid: PoolUuid,
    mut meta_dev: LinearDev,
    device: Device,
    spare_segments: &[(Sectors, Sectors)],
) -> StratisResult<LinearDev> {
    let (dm_name, dm_uuid) = format_flex_ids(pool_uuid, FlexRole::ThinMetaSpare);
    let mut new_meta_dev = LinearDev::setup(
        get_dm(),
        &dm_name,
        Some(&dm_uuid),
        segs_to_table(device, spare_segments),
    )?;

    thin_repair(&meta_dev.devnode(), &new_meta_dev.devnode())?;

    let name = meta_dev.name().to_owned();
    meta_dev.teardown(get_dm())?;
    new_meta_dev.set_name(get_dm(), &name)?;

    Ok(new_meta_dev)
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::io::{BufWriter, Read, Write};
    use std::path::Path;

    use nix::mount::{mount, umount, MsFlags};
    use tempfile;
    use uuid::Uuid;

    use devicemapper::{Bytes, SECTOR_SIZE};

    use super::super::super::backstore::MIN_MDA_SECTORS;
    use super::super::super::cmd;
    use super::super::super::device::SyncAll;
    use super::super::super::tests::{loopbacked, real};

    use super::super::filesystem::{fs_usage, FILESYSTEM_LOWATER};

    use super::*;

    const BYTES_PER_WRITE: usize = 2 * IEC::Ki as usize * SECTOR_SIZE as usize;

    /// Verify that a full pool extends properly when additional space is added.
    fn test_full_pool(paths: &[&Path]) {
        let pool_uuid = Uuid::new_v4();
        devlinks::setup_dev_path().unwrap();
        devlinks::cleanup_devlinks(Vec::new().into_iter());
        let (first_path, remaining_paths) = paths.split_at(1);
        let mut backstore = Backstore::initialize(pool_uuid, &first_path, MIN_MDA_SECTORS).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name);
        let fs_uuid = pool.create_filesystem(pool_uuid, pool_name, "stratis_test_filesystem", None)
            .unwrap();
        let write_buf = &[8u8; BYTES_PER_WRITE];
        let source_tmp_dir = tempfile::Builder::new()
            .prefix("stratis_testing")
            .tempdir()
            .unwrap();
        {
            // to allow mutable borrow of pool
            let (_, filesystem) = pool.get_filesystem_by_uuid(fs_uuid).unwrap();
            mount(
                Some(&filesystem.devnode()),
                source_tmp_dir.path(),
                Some("xfs"),
                MsFlags::empty(),
                None as Option<&str>,
            ).unwrap();
            let file_path = source_tmp_dir.path().join("stratis_test.txt");
            let mut f = BufWriter::with_capacity(
                IEC::Mi as usize,
                OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(file_path)
                    .unwrap(),
            );
            // Write the write_buf until the pool is full
            loop {
                match pool.thin_pool.status(get_dm()).unwrap() {
                    ThinPoolStatus::Working(_) => {
                        f.write_all(write_buf).unwrap();
                        if f.sync_all().is_err() {
                            break;
                        }
                    }
                    ThinPoolStatus::Fail => panic!("ThinPoolStatus::Fail  Expected working."),
                }
            }
        }
        match pool.thin_pool.status(get_dm()).unwrap() {
            ThinPoolStatus::Working(ref status) => {
                assert_eq!(
                    status.summary,
                    ThinPoolStatusSummary::OutOfSpace,
                    "Expected full pool"
                );
            }
            ThinPoolStatus::Fail => panic!("ThinPoolStatus::Fail  Expected working/full."),
        };

        // Clear anything that may be on the remaining paths to ensure we can add them
        // to the pool
        for p in remaining_paths {
            wipe_sectors(p, Sectors(0), MIN_MDA_SECTORS).unwrap();
        }
        cmd::udev_settle().unwrap();

        // Add block devices to the pool and run check() to extend
        backstore.add_datadevs(pool_uuid, &remaining_paths).unwrap();
        pool.check(pool_uuid, &mut backstore).unwrap();
        // Verify the pool is back in a Good state
        match pool.thin_pool.status(get_dm()).unwrap() {
            ThinPoolStatus::Working(ref status) => {
                assert_eq!(
                    status.summary,
                    ThinPoolStatusSummary::Good,
                    "Expected pool to be restored to good state"
                );
            }
            ThinPoolStatus::Fail => panic!("ThinPoolStatus::Fail.  Expected working/good."),
        };
    }

    #[test]
    pub fn loop_test_full_pool() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Exactly(2, Some(Bytes(IEC::Gi).sectors())),
            test_full_pool,
        );
    }

    #[test]
    pub fn real_test_full_pool() {
        real::test_with_spec(
            real::DeviceLimits::Exactly(
                2,
                Some(Bytes(IEC::Gi).sectors()),
                Some(Bytes(IEC::Gi * 4).sectors()),
            ),
            test_full_pool,
        );
    }

    /// Verify a snapshot has the same files and same contents as the origin.
    fn test_filesystem_snapshot(paths: &[&Path]) {
        let pool_uuid = Uuid::new_v4();
        devlinks::cleanup_devlinks(Vec::new().into_iter());
        let mut backstore = Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name);
        let fs_uuid = pool.create_filesystem(pool_uuid, pool_name, "stratis_test_filesystem", None)
            .unwrap();

        let write_buf = &[8u8; SECTOR_SIZE];
        let file_count = 10;

        let source_tmp_dir = tempfile::Builder::new()
            .prefix("stratis_testing")
            .tempdir()
            .unwrap();
        {
            // to allow mutable borrow of pool
            let (_, filesystem) = pool.get_filesystem_by_uuid(fs_uuid).unwrap();
            mount(
                Some(&filesystem.devnode()),
                source_tmp_dir.path(),
                Some("xfs"),
                MsFlags::empty(),
                None as Option<&str>,
            ).unwrap();
            for i in 0..file_count {
                let file_path = source_tmp_dir.path().join(format!("stratis_test{}.txt", i));
                let mut f = BufWriter::with_capacity(
                    IEC::Mi as usize,
                    OpenOptions::new()
                        .create(true)
                        .write(true)
                        .open(file_path)
                        .unwrap(),
                );
                f.write_all(write_buf).unwrap();
                f.sync_all().unwrap();
            }
        }

        // Double the size of the data device. The space initially allocated
        // to a pool is close to consumed by the filesystem and few files
        // written above. If we attempt to update the UUID on the snapshot
        // without expanding the pool, the pool will go into out-of-data-space
        // (queue IO) mode, causing the test to fail.
        pool.extend_thin_data_device(
            pool_uuid,
            &mut backstore,
            datablocks_to_sectors(INITIAL_DATA_SIZE),
        ).unwrap();

        let (_, snapshot_filesystem) =
            pool.snapshot_filesystem(pool_uuid, pool_name, fs_uuid, "test_snapshot")
                .unwrap();
        let mut read_buf = [0u8; SECTOR_SIZE];
        let snapshot_tmp_dir = tempfile::Builder::new()
            .prefix("stratis_testing")
            .tempdir()
            .unwrap();
        {
            mount(
                Some(&snapshot_filesystem.devnode()),
                snapshot_tmp_dir.path(),
                Some("xfs"),
                MsFlags::empty(),
                None as Option<&str>,
            ).unwrap();
            for i in 0..file_count {
                let file_path = snapshot_tmp_dir
                    .path()
                    .join(format!("stratis_test{}.txt", i));
                let mut f = OpenOptions::new().read(true).open(file_path).unwrap();
                f.read(&mut read_buf).unwrap();
                assert_eq!(read_buf[0..SECTOR_SIZE], write_buf[0..SECTOR_SIZE]);
            }
        }
    }

    #[test]
    pub fn loop_test_filesystem_snapshot() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(2, 3, None),
            test_filesystem_snapshot,
        );
    }

    #[test]
    pub fn real_test_filesystem_snapshot() {
        real::test_with_spec(
            real::DeviceLimits::AtLeast(2, None, None),
            test_filesystem_snapshot,
        );
    }

    /// Verify that a filesystem rename causes the filesystem metadata to be
    /// updated.
    fn test_filesystem_rename(paths: &[&Path]) {
        let name1 = "name1";
        let name2 = "name2";

        let pool_uuid = Uuid::new_v4();
        devlinks::cleanup_devlinks(Vec::new().into_iter());
        let mut backstore = Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name);
        let fs_uuid = pool.create_filesystem(pool_uuid, pool_name, &name1, None)
            .unwrap();

        let action = pool.rename_filesystem(pool_name, fs_uuid, name2).unwrap();
        assert_eq!(action, RenameAction::Renamed);
        let flexdevs: FlexDevsSave = pool.record();
        let thinpoolsave: ThinPoolDevSave = pool.record();
        pool.teardown().unwrap();

        let pool = ThinPool::setup(pool_uuid, &thinpoolsave, &flexdevs, &backstore).unwrap();

        assert_eq!(&*pool.get_filesystem_by_uuid(fs_uuid).unwrap().0, name2);
    }

    #[test]
    pub fn loop_test_filesystem_rename() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(1, 3, None),
            test_filesystem_rename,
        );
    }

    #[test]
    pub fn real_test_filesystem_rename() {
        real::test_with_spec(
            real::DeviceLimits::AtLeast(1, None, None),
            test_filesystem_rename,
        );
    }

    /// Verify that setting up a pool when the pool has not been previously torn
    /// down does not fail. Clutter the original pool with a filesystem with
    /// some data on it.
    fn test_pool_setup(paths: &[&Path]) {
        let pool_uuid = Uuid::new_v4();
        devlinks::cleanup_devlinks(Vec::new().into_iter());
        let mut backstore = Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name);
        let fs_uuid = pool.create_filesystem(pool_uuid, pool_name, "fsname", None)
            .unwrap();

        let tmp_dir = tempfile::Builder::new()
            .prefix("stratis_testing")
            .tempdir()
            .unwrap();
        let new_file = tmp_dir.path().join("stratis_test.txt");
        {
            let (_, fs) = pool.get_filesystem_by_uuid(fs_uuid).unwrap();
            mount(
                Some(&fs.devnode()),
                tmp_dir.path(),
                Some("xfs"),
                MsFlags::empty(),
                None as Option<&str>,
            ).unwrap();
            writeln!(
                &OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(new_file)
                    .unwrap(),
                "data"
            ).unwrap();
        }
        let thinpooldevsave: ThinPoolDevSave = pool.record();

        let new_pool =
            ThinPool::setup(pool_uuid, &thinpooldevsave, &pool.record(), &backstore).unwrap();

        assert!(new_pool.get_filesystem_by_uuid(fs_uuid).is_some());
    }

    #[test]
    pub fn loop_test_pool_setup() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(1, 3, None), test_pool_setup);
    }

    #[test]
    pub fn real_test_pool_setup() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1, None, None), test_pool_setup);
    }
    /// Verify that destroy_filesystems actually deallocates the space
    /// from the thinpool, by attempting to reinstantiate it using the
    /// same thin id and verifying that it fails.
    fn test_thindev_destroy(paths: &[&Path]) -> () {
        let pool_uuid = Uuid::new_v4();
        devlinks::cleanup_devlinks(Vec::new().into_iter());
        let mut backstore = Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();
        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name);
        let fs_name = "stratis_test_filesystem";
        let fs_uuid = pool.create_filesystem(pool_uuid, pool_name, &fs_name, None)
            .unwrap();
        pool.destroy_filesystem(pool_name, fs_uuid).unwrap();
        let flexdevs: FlexDevsSave = pool.record();
        let thinpooldevsave: ThinPoolDevSave = pool.record();
        pool.teardown().unwrap();

        // Check that destroyed fs is not present in MDV. If the record
        // had been left on the MDV that didn't match a thin_id in the
        // thinpool, ::setup() will fail.
        let pool = ThinPool::setup(pool_uuid, &thinpooldevsave, &flexdevs, &backstore).unwrap();

        assert!(pool.get_filesystem_by_uuid(fs_uuid).is_none());
    }

    #[test]
    pub fn loop_test_thindev_destroy() {
        // This test requires more than 1 GiB.
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(2, 3, None),
            test_thindev_destroy,
        );
    }

    #[test]
    pub fn real_test_thindev_destroy() {
        real::test_with_spec(
            real::DeviceLimits::AtLeast(1, None, None),
            test_thindev_destroy,
        );
    }

    /// Verify that the meta device backing a ThinPool is expanded when meta
    /// utilization exceeds the kernel-set meta lowater mark, by creating a
    /// ThinPool with a meta device of such a small size that we've determined
    /// it will definitely be smaller than the meta lowater value.
    fn test_meta_expand(paths: &[&Path]) -> () {
        let pool_uuid = Uuid::new_v4();
        let small_meta_size = MetaBlocks(16);
        let mut backstore = Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS).unwrap();
        // Create a ThinPool with a very small meta device.
        let mut thin_pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams {
                meta_size: small_meta_size,
                ..Default::default()
            },
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();

        match thin_pool.thin_pool.status(get_dm()).unwrap() {
            ThinPoolStatus::Working(ref status) => {
                let usage = &status.usage;
                assert_eq!(usage.total_meta, small_meta_size);
                assert!(usage.used_meta > MetaBlocks(0));
            }
            ThinPoolStatus::Fail => panic!("thin_pool.status() failed"),
        }
        // The meta device is smaller than meta lowater, so it should be expanded
        // in the thin_pool.check() call.
        thin_pool.check(pool_uuid, &mut backstore).unwrap();
        match thin_pool.thin_pool.status(get_dm()).unwrap() {
            ThinPoolStatus::Working(ref status) => {
                let usage = &status.usage;
                // validate that the meta has been expanded.
                assert!(usage.total_meta > small_meta_size);
            }
            ThinPoolStatus::Fail => panic!("thin_pool.status() failed"),
        }
    }

    #[test]
    pub fn loop_test_meta_expand() {
        // This test requires more than 1 GiB.
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(2, 3, None),
            test_meta_expand,
        );
    }

    #[test]
    pub fn real_test_meta_expand() {
        real::test_with_spec(
            real::DeviceLimits::Range(2, 3, None, None),
            test_meta_expand,
        );
    }

    /// Verify that thinpool is expanded to the point where more data can
    /// be written to it than its original size.
    /// 1. Initialize the pool.
    /// 2. Make a filesystem.
    /// 3. Write to a file until the amount left is definitely at or below the
    /// low water mark.
    /// 4. Expand the thin pool data device by twice the low water amount.
    /// 5. Continue to write until the amount written exceeds the original
    /// size of the data device.
    /// 6. Run xfs_repair to verify success.
    fn test_thinpool_expand(paths: &[&Path]) -> () {
        let pool_uuid = Uuid::new_v4();
        devlinks::cleanup_devlinks(Vec::new().into_iter());
        let mut backstore = Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();
        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name);
        let fs_name = "stratis_test_filesystem";
        let fs_uuid = pool.create_filesystem(pool_uuid, pool_name, fs_name, None)
            .unwrap();

        let fs_devnode = pool.get_filesystem_by_uuid(fs_uuid).unwrap().1.devnode();
        let tmp_dir = tempfile::Builder::new()
            .prefix("stratis_testing")
            .tempdir()
            .unwrap();
        mount(
            Some(&fs_devnode),
            tmp_dir.path(),
            Some("xfs"),
            MsFlags::empty(),
            None as Option<&str>,
        ).unwrap();

        // This is a constant, since only explicit actions of the ThinPool
        // can change it.
        let data_lowater = pool.thin_pool.table().table.params.low_water_mark;

        let (total_data, used_data) = match pool.thin_pool.status(get_dm()).unwrap() {
            ThinPoolStatus::Working(ref status) => {
                (status.usage.total_data, status.usage.used_data)
            }
            _ => panic!("Pool status indicates already failed."),
        };

        // used data is not 0, because the filesystem has been created
        assert!(used_data != DataBlocks(0));

        {
            let mut f = BufWriter::with_capacity(
                IEC::Mi as usize,
                OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(tmp_dir.path().join("stratis_test.txt"))
                    .unwrap(),
            );

            // Keep going until pool gets to data lowater
            let buf = &[1u8; SECTOR_SIZE];
            let mut amount_written = Sectors(0);
            let total_data_sectors = datablocks_to_sectors(total_data);
            let data_lowater_sectors = datablocks_to_sectors(data_lowater);
            let used_data_sectors = datablocks_to_sectors(used_data);
            let total_remaining = total_data_sectors - used_data_sectors;
            while total_remaining - amount_written >= data_lowater_sectors {
                f.write_all(buf).unwrap();
                amount_written += Sectors(1);
            }

            // Force the contents of the buffer to the thinpool data device.
            f.sync_all().unwrap();

            // Verify at or below data lowater, but pool still good.
            match pool.thin_pool.status(get_dm()).unwrap() {
                ThinPoolStatus::Working(ref status) => {
                    assert_eq!(status.summary, ThinPoolStatusSummary::Good);
                    assert!(status.usage.total_data - status.usage.used_data <= data_lowater);
                }
                _ => panic!("Pool status indicates already failed."),
            }

            // Extend the thin pool by twice the data lowater amount
            let extend_amount = 2u64 * data_lowater_sectors;
            let amount_extended =
                pool.extend_thin_data_device(pool_uuid, &mut backstore, extend_amount)
                    .unwrap();

            assert_eq!(
                amount_extended, extend_amount,
                "The backstore was set up without enough space to properly extend the pool, this test has bad initial conditions"
            );

            let (new_total_data, new_used_data) = match pool.thin_pool.status(get_dm()).unwrap() {
                ThinPoolStatus::Working(ref status) => {
                    (status.usage.total_data, status.usage.used_data)
                }
                _ => panic!("Pool status indicates already failed."),
            };

            // The amount used after all that writing ought to be less than
            // the original total, because it is impossible for it to be more
            // as nothing was written after the pool was extended.
            assert!(new_used_data < total_data);
            assert_eq!(new_used_data, used_data);

            // If the extension worked, the new size should be the old, plus
            // the extension.
            assert_eq!(
                datablocks_to_sectors(new_total_data),
                total_data_sectors + amount_extended
            );

            // Keep writing until amount written exceeds the original total
            // data device size by data lowater.
            let limit = datablocks_to_sectors(total_data + data_lowater - new_used_data);

            let mut amount_written = Sectors(0);
            while amount_written < limit {
                f.write_all(buf).unwrap();
                amount_written += Sectors(1);
            }

            // synching should succeed.
            assert!(f.sync_all().is_ok());

            // Verify amount written
            match pool.thin_pool.status(get_dm()).unwrap() {
                ThinPoolStatus::Working(ref status) => {
                    assert_eq!(status.summary, ThinPoolStatusSummary::Good);
                    assert!(status.usage.used_data > new_used_data);
                    assert!(status.usage.used_data > total_data);
                }
                _ => panic!("Pool status indicates already failed."),
            }
        }
        umount(tmp_dir.path()).unwrap();
        assert!(cmd::xfs_repair(&fs_devnode).is_ok());
    }

    #[test]
    pub fn loop_test_thinpool_expand() {
        // This test requires more than 1 GiB.
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(2, 3, None),
            test_thinpool_expand,
        );
    }

    #[test]
    pub fn real_test_thinpool_expand() {
        real::test_with_spec(
            real::DeviceLimits::AtLeast(1, None, None),
            test_thinpool_expand,
        );
    }

    /// Verify that the logical space allocated to a filesystem is expanded when
    /// the number of sectors written to the filesystem causes the free space to
    /// dip below the FILESYSTEM_LOWATER mark. Verify that the space has been
    /// expanded by calling filesystem.check() then looking at the total space
    /// compared to the original size.
    fn test_xfs_expand(paths: &[&Path]) -> () {
        let pool_uuid = Uuid::new_v4();
        devlinks::cleanup_devlinks(Vec::new().into_iter());
        let mut backstore = Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();

        // Create a filesystem as small as possible.  Allocate 1 MiB bigger than
        // the low water mark.
        let fs_size = FILESYSTEM_LOWATER + Bytes(IEC::Mi).sectors();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name);
        let fs_name = "stratis_test_filesystem";
        let fs_uuid = pool.create_filesystem(pool_uuid, pool_name, fs_name, Some(fs_size))
            .unwrap();

        // Braces to ensure f is closed before destroy and the borrow of
        // pool is complete
        {
            let filesystem = pool.get_mut_filesystem_by_uuid(fs_uuid).unwrap().1;
            // Write 2 MiB of data. The filesystem's free space is now 1 MiB
            // below FILESYSTEM_LOWATER.
            let write_size = Bytes(IEC::Mi * 2).sectors();
            let tmp_dir = tempfile::Builder::new()
                .prefix("stratis_testing")
                .tempdir()
                .unwrap();
            mount(
                Some(&filesystem.devnode()),
                tmp_dir.path(),
                Some("xfs"),
                MsFlags::empty(),
                None as Option<&str>,
            ).unwrap();
            let buf = &[1u8; SECTOR_SIZE];
            for i in 0..*write_size {
                let file_path = tmp_dir.path().join(format!("stratis_test{}.txt", i));
                let mut f = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(file_path)
                    .unwrap();
                if f.write_all(buf).is_err() {
                    break;
                }
            }
            let (orig_fs_total_bytes, _) = fs_usage(&tmp_dir.path()).unwrap();
            // Simulate handling a DM event by running a filesystem check.
            filesystem.check().unwrap();
            let (fs_total_bytes, _) = fs_usage(&tmp_dir.path()).unwrap();
            assert!(fs_total_bytes > orig_fs_total_bytes);
            umount(tmp_dir.path()).unwrap();
        }
    }

    #[test]
    pub fn loop_test_xfs_expand() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(1, 3, None), test_xfs_expand);
    }

    #[test]
    pub fn real_test_xfs_expand() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1, None, None), test_xfs_expand);
    }

    /// Just suspend and resume the device and make sure it doesn't crash.
    /// Suspend twice in succession and then resume twice in succession
    /// to check idempotency.
    fn test_suspend_resume(paths: &[&Path]) {
        let pool_uuid = Uuid::new_v4();
        devlinks::cleanup_devlinks(Vec::new().into_iter());
        let mut backstore = Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name);
        pool.create_filesystem(pool_uuid, pool_name, "stratis_test_filesystem", None)
            .unwrap();

        pool.suspend().unwrap();
        pool.suspend().unwrap();
        pool.resume().unwrap();
        pool.resume().unwrap();
    }

    #[test]
    pub fn loop_test_suspend_resume() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(1, 3, None),
            test_suspend_resume,
        );
    }

    #[test]
    pub fn real_test_suspend_resume() {
        real::test_with_spec(
            real::DeviceLimits::AtLeast(1, None, None),
            test_suspend_resume,
        );
    }

    /// Set up thinpool and backstore. Set up filesystem and write to it.
    /// Add cachedev to backstore, causing cache to be built.
    /// Update device on self. Read written bits from filesystem
    /// presented on cache device.
    fn test_set_device(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let pool_uuid = Uuid::new_v4();
        devlinks::cleanup_devlinks(Vec::new().into_iter());
        let mut backstore = Backstore::initialize(pool_uuid, paths2, MIN_MDA_SECTORS).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name);
        let fs_uuid = pool.create_filesystem(pool_uuid, pool_name, "stratis_test_filesystem", None)
            .unwrap();

        let tmp_dir = tempfile::Builder::new()
            .prefix("stratis_testing")
            .tempdir()
            .unwrap();
        let new_file = tmp_dir.path().join("stratis_test.txt");
        let bytestring = b"some bytes";
        {
            let (_, fs) = pool.get_filesystem_by_uuid(fs_uuid).unwrap();
            mount(
                Some(&fs.devnode()),
                tmp_dir.path(),
                Some("xfs"),
                MsFlags::empty(),
                None as Option<&str>,
            ).unwrap();
            OpenOptions::new()
                .create(true)
                .write(true)
                .open(&new_file)
                .unwrap()
                .write(bytestring)
                .unwrap();
        }
        let filesystem_saves = pool.mdv.filesystems().unwrap();
        assert_eq!(filesystem_saves.len(), 1);
        assert_eq!(
            filesystem_saves
                .first()
                .expect("filesystem_saves().len == 1")
                .uuid,
            fs_uuid
        );

        pool.suspend().unwrap();
        let old_device = backstore
            .device()
            .expect("Space already allocated from backstore, backstore must have device");
        backstore.add_cachedevs(pool_uuid, paths1).unwrap();
        let new_device = backstore
            .device()
            .expect("Space already allocated from backstore, backstore must have device");
        assert_ne!(old_device, new_device);
        pool.set_device(new_device).unwrap();
        pool.resume().unwrap();

        let mut buf = [0u8; 10];
        {
            OpenOptions::new()
                .read(true)
                .open(&new_file)
                .unwrap()
                .read(&mut buf)
                .unwrap();
        }
        assert_eq!(&buf, bytestring);

        let filesystem_saves = pool.mdv.filesystems().unwrap();
        assert_eq!(filesystem_saves.len(), 1);
        assert_eq!(
            filesystem_saves
                .first()
                .expect("filesystem_saves().len == 1")
                .uuid,
            fs_uuid
        );
    }

    #[test]
    pub fn loop_test_set_device() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(2, 3, None), test_set_device);
    }

    #[test]
    pub fn real_test_set_device() {
        real::test_with_spec(real::DeviceLimits::AtLeast(2, None, None), test_set_device);
    }
}

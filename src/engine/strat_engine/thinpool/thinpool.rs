// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle management of a pool's thinpool device.

use std;
use std::borrow::BorrowMut;

use uuid::Uuid;

use devicemapper as dm;
use devicemapper::{
    device_exists, Bytes, DataBlocks, Device, DmDevice, DmName, DmNameBuf, FlakeyTargetParams,
    LinearDev, LinearDevTargetParams, LinearTargetParams, MetaBlocks, Sectors, TargetLine,
    ThinDevId, ThinPoolDev, ThinPoolStatusSummary, IEC,
};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::devlinks;
use super::super::super::engine::Filesystem;
use super::super::super::event::{get_engine_listener_list, EngineEvent};
use super::super::super::structures::Table;
use super::super::super::types::{
    FilesystemUuid, FreeSpaceState, MaybeDbusPath, Name, PoolState, PoolUuid, RenameAction,
};

use super::super::backstore::Backstore;
use super::super::cmd::{thin_check, thin_repair};
use super::super::device::wipe_sectors;
use super::super::dm::get_dm;
use super::super::dmnames::{
    format_flex_ids, format_thin_ids, format_thinpool_ids, FlexRole, ThinPoolRole, ThinRole,
};
use super::super::serde_structs::{FlexDevsSave, Recordable, ThinPoolDevSave};
use super::super::set_write_throttling;

use super::filesystem::{FilesystemStatus, StratFilesystem};
use super::mdv::MetadataVol;
use super::thinids::ThinDevIdPool;

pub const DATA_BLOCK_SIZE: Sectors = Sectors(2 * IEC::Ki);
const DATA_LOWATER: DataBlocks = DataBlocks(512);
const META_LOWATER_FALLBACK: MetaBlocks = MetaBlocks(512);

const INITIAL_META_SIZE: MetaBlocks = MetaBlocks(4 * IEC::Ki);
pub const INITIAL_DATA_SIZE: DataBlocks = DataBlocks(768);
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

/// Calculate new low water based on the current thinpool data device size
/// and the amount of unused sectors available in the cap device.
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
    meta_segments: Vec<(Sectors, Sectors)>,
    meta_spare_segments: Vec<(Sectors, Sectors)>,
    data_segments: Vec<(Sectors, Sectors)>,
    mdv_segments: Vec<(Sectors, Sectors)>,
    id_gen: ThinDevIdPool,
    filesystems: Table<StratFilesystem>,
    mdv: MetadataVol,
    /// The single DM device that the backstore presents as its upper-most
    /// layer. All DM components obtain their storage from this layer.
    /// The device will change if the backstore adds or removes a cache.
    backstore_device: Device,
    pool_state: PoolState,
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
        wipe_sectors(&meta_dev.devnode(), Sectors(0), meta_dev.size())?;

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
                sectors_to_datablocks(backstore.available()),
                free_space_state,
            ),
        )?;

        Ok(ThinPool {
            thin_pool: thinpool_dev,
            meta_segments: vec![meta_segments],
            meta_spare_segments: vec![spare_segments],
            data_segments: vec![data_segments],
            mdv_segments: vec![mdv_segments],
            id_gen: ThinDevIdPool::new_from_ids(&[]),
            filesystems: Table::default(),
            mdv,
            backstore_device,
            pool_state: PoolState::Good,
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
                sectors_to_datablocks(backstore.available()),
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
            meta_segments,
            meta_spare_segments: spare_segments,
            data_segments,
            mdv_segments,
            id_gen: ThinDevIdPool::new_from_ids(&thin_ids),
            filesystems: fs_table,
            mdv,
            backstore_device,
            pool_state: PoolState::Good,
            free_space_state,
            dbus_path: MaybeDbusPath(None),
        })
    }

    /// Run status checks and take actions on the thinpool and its components.
    /// Returns a bool communicating if a configuration change requiring a
    /// metadata save has been made.
    pub fn check(&mut self, pool_uuid: PoolUuid, backstore: &mut Backstore) -> StratisResult<bool> {
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
                    Bytes(IEC::Gi).sectors()
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

        let thinpool: dm::ThinPoolStatus = self.thin_pool.status(get_dm())?;
        match thinpool {
            dm::ThinPoolStatus::Working(ref status) => {
                match status.summary {
                    ThinPoolStatusSummary::Good => {}
                    ThinPoolStatusSummary::ReadOnly => {
                        error!("Thinpool readonly! -> BAD");
                        self.set_state(PoolState::Bad);
                    }
                    ThinPoolStatusSummary::OutOfSpace => {
                        error!("Thinpool out of space! -> BAD");
                        self.set_state(PoolState::Bad);
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
                    match self.extend_thin_meta_device(pool_uuid, backstore, request) {
                        Ok(extend_size) => {
                            info!("Extended thin meta device by {}", extend_size);

                            should_save = true;
                        }
                        Err(err) => {
                            error!("Thinpool meta extend failed! -> BAD: reason {:?}", err);
                            self.set_state(PoolState::Bad);
                        }
                    }
                }

                let extend_size = {
                    match calculate_extension_request(
                        datablocks_to_sectors(usage.total_data),
                        datablocks_to_sectors(usage.used_data),
                        datablocks_to_sectors(self.thin_pool.table().table.params.low_water_mark),
                        true,
                    ) {
                        None => DataBlocks(0),
                        Some(request) => match self.extend_thin_data_device(
                            pool_uuid, backstore, request,
                        ) {
                            Ok(Sectors(0)) => {
                                warn!("data device fully extended, cannot extend further");
                                DataBlocks(0)
                            }
                            Ok(extend_size) => {
                                info!("Extended thin data device by {}", extend_size);
                                should_save = true;
                                sectors_to_datablocks(extend_size)
                            }
                            Err(err) => {
                                error!("Thinpool data extend failed! -> BAD: reason: {:?}", err);
                                self.set_state(PoolState::Bad);
                                DataBlocks(0)
                            }
                        },
                    }
                };

                let current_total = usage.total_data + extend_size;

                // Update pool space state
                self.free_space_state = self.free_space_check(
                    usage.used_data,
                    current_total + sectors_to_datablocks(backstore.available()) - usage.used_data,
                )?;

                // Trigger next event depending on pool space state
                let lowater = calc_lowater(
                    current_total,
                    sectors_to_datablocks(backstore.available()),
                    self.free_space_state,
                );

                self.thin_pool.set_low_water_mark(get_dm(), lowater)?;
                self.resume()?;
            }
            dm::ThinPoolStatus::Fail => {
                error!("Thinpool status is `fail` -> BAD");
                self.set_state(PoolState::Bad);
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

        let new_state = match overall_used_pct {
            0...SPACE_WARN_PCT => FreeSpaceState::Good,
            SPACE_WARN_PCT...SPACE_CRIT_PCT => FreeSpaceState::Warn,
            _ => FreeSpaceState::Crit,
        };

        if self.free_space_state != new_state {
            info!(
                "Prev space state: {:?} New space state: {:?}",
                self.free_space_state, new_state
            );

            // TODO: Dbus signal
        }

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
        info!(
            "Attempting to extend thinpool data device belonging to pool {} by {}",
            pool_uuid, extend_size,
        );

        ThinPool::extend_thin_sub_device(
            pool_uuid,
            &mut self.thin_pool,
            backstore,
            extend_size,
            DATA_BLOCK_SIZE,
            &mut self.data_segments,
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
        info!(
            "Attempting to extend thinpool meta device belonging to pool {} by {}",
            pool_uuid, extend_size,
        );

        ThinPool::extend_thin_sub_device(
            pool_uuid,
            &mut self.thin_pool,
            backstore,
            extend_size,
            MetaBlocks(1).sectors(),
            &mut self.meta_segments,
            false,
        )
    }

    /// Extend the thinpool's data or meta devices. The result is the value
    /// by which the device is extended which may be less than the requested
    /// amount. It is guaranteed that the returned amount is a multiple of the
    /// modulus value. Sets existing_segs to the new value that specifies the
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
        if let Some(region) = backstore.request(pool_uuid, extend_size, modulus)? {
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
            let err_msg = format!(
                "Insufficient space to accomodate request for at least {}",
                modulus
            );
            Err(StratisError::Engine(ErrorEnum::Error, err_msg))
        }
    }

    /// The number of physical sectors in use, that is, unavailable for storage
    /// of additional user data, by this pool.
    // This includes all the sectors being held as spares for the meta device,
    // all the sectors allocated to the meta data device, and all the sectors
    // in use on the data device.
    pub fn total_physical_used(&self) -> StratisResult<Sectors> {
        let data_dev_used = match self.thin_pool.status(get_dm())? {
            dm::ThinPoolStatus::Working(ref status) => {
                datablocks_to_sectors(status.usage.used_data)
            }
            _ => {
                let err_msg = "thin pool failed, could not obtain usage";
                return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg.into()));
            }
        };

        let spare_total = self.meta_spare_segments.iter().map(|s| s.1).sum();

        let meta_dev_total = self.thin_pool.meta_dev().size();

        let mdv_total = self.mdv_segments.iter().map(|s| s.1).sum();

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

    #[allow(dead_code)]
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
        let (fs_uuid, new_filesystem) =
            StratFilesystem::initialize(pool_uuid, &self.thin_pool, size, self.id_gen.new_id()?)?;
        let name = Name::new(name.to_owned());
        self.mdv.save_fs(&name, fs_uuid, &new_filesystem)?;
        devlinks::filesystem_added(pool_name, &name, &new_filesystem.devnode())?;
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
        devlinks::filesystem_added(pool_name, &new_fs_name, &new_filesystem.devnode())?;
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
                    if let Err(err) = devlinks::filesystem_removed(pool_name, &fs_name) {
                        error!("Could not remove devlinks for fs with UUID {} and name {} belonging to pool {}, reason: {:?}",
                               uuid,
                               fs_name,
                               pool_name,
                               err);
                    }
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
            devlinks::filesystem_renamed(pool_name, &old_name, &new_name)?;
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

impl Recordable<FlexDevsSave> for ThinPool {
    fn record(&self) -> FlexDevsSave {
        FlexDevsSave {
            meta_dev: self.mdv_segments.to_vec(),
            thin_meta_dev: self.meta_segments.to_vec(),
            thin_data_dev: self.data_segments.to_vec(),
            thin_meta_dev_spare: self.meta_spare_segments.to_vec(),
        }
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
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::path::Path;

    use nix::mount::{mount, umount, MsFlags};
    use tempfile;
    use uuid::Uuid;

    use devicemapper::{Bytes, SECTOR_SIZE};

    use super::super::super::super::types::BlockDevTier;

    use super::super::super::backstore::MIN_MDA_SECTORS;
    use super::super::super::tests::{loopbacked, real};

    use super::super::filesystem::{fs_usage, FILESYSTEM_LOWATER};

    use super::*;

    const BYTES_PER_WRITE: usize = 2 * IEC::Ki as usize * SECTOR_SIZE as usize;

    /// Verify that a full pool extends properly when additional space is added.
    fn test_full_pool(paths: &[&Path]) {
        let pool_uuid = Uuid::new_v4();
        devlinks::setup_dev_path().unwrap();
        devlinks::setup_devlinks(Vec::new().into_iter()).unwrap();
        let (first_path, remaining_paths) = paths.split_at(1);
        let mut backstore =
            Backstore::initialize(pool_uuid, &first_path, MIN_MDA_SECTORS, false).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
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
            let mut f: File = OpenOptions::new()
                .create(true)
                .write(true)
                .open(file_path)
                .unwrap();
            // Write the write_buf until the pool is full
            loop {
                let status: dm::ThinPoolStatus = pool.thin_pool.status(get_dm()).unwrap();
                match status {
                    dm::ThinPoolStatus::Working(_) => {
                        f.write_all(write_buf).unwrap();
                        if f.sync_data().is_err() {
                            break;
                        }
                    }
                    dm::ThinPoolStatus::Fail => panic!("ThinPoolStatus::Fail  Expected working."),
                }
            }
        }
        match pool.thin_pool.status(get_dm()).unwrap() {
            dm::ThinPoolStatus::Working(ref status) => {
                assert!(
                    status.summary == ThinPoolStatusSummary::OutOfSpace,
                    "Expected full pool"
                );
            }
            dm::ThinPoolStatus::Fail => panic!("ThinPoolStatus::Fail  Expected working/full."),
        };
        // Add block devices to the pool and run check() to extend
        backstore
            .add_blockdevs(pool_uuid, &remaining_paths, BlockDevTier::Data, true)
            .unwrap();
        pool.check(pool_uuid, &mut backstore).unwrap();
        // Verify the pool is back in a Good state
        match pool.thin_pool.status(get_dm()).unwrap() {
            dm::ThinPoolStatus::Working(ref status) => {
                assert!(
                    status.summary == ThinPoolStatusSummary::Good,
                    "Expected pool to be restored to good state"
                );
            }
            dm::ThinPoolStatus::Fail => panic!("ThinPoolStatus::Fail.  Expected working/good."),
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
        devlinks::setup_devlinks(Vec::new().into_iter()).unwrap();
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
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
                let mut f = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(file_path)
                    .unwrap();
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
        devlinks::setup_devlinks(Vec::new().into_iter()).unwrap();
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
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
        devlinks::setup_devlinks(Vec::new().into_iter()).unwrap();
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
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
        devlinks::setup_devlinks(Vec::new().into_iter()).unwrap();
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();
        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
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
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).unwrap();
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
            dm::ThinPoolStatus::Working(ref status) => {
                let usage = &status.usage;
                assert_eq!(usage.total_meta, small_meta_size);
                assert!(usage.used_meta > MetaBlocks(0));
            }
            dm::ThinPoolStatus::Fail => panic!("thin_pool.status() failed"),
        }
        // The meta device is smaller than meta lowater, so it should be expanded
        // in the thin_pool.check() call.
        thin_pool.check(pool_uuid, &mut backstore).unwrap();
        match thin_pool.thin_pool.status(get_dm()).unwrap() {
            dm::ThinPoolStatus::Working(ref status) => {
                let usage = &status.usage;
                // validate that the meta has been expanded.
                assert!(usage.total_meta > small_meta_size);
            }
            dm::ThinPoolStatus::Fail => panic!("thin_pool.status() failed"),
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

    /// Verify that the physical space allocated to a pool is expanded when
    /// the number of sectors written to a thin-dev in the pool exceeds the
    /// INITIAL_DATA_SIZE.  If we are able to write more sectors to the
    /// filesystem than are initially allocated to the pool, the pool must
    /// have been expanded.
    fn test_thinpool_expand(paths: &[&Path]) -> () {
        let pool_uuid = Uuid::new_v4();
        devlinks::setup_devlinks(Vec::new().into_iter()).unwrap();
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();
        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
        let fs_name = "stratis_test_filesystem";
        let fs_uuid = pool.create_filesystem(pool_uuid, pool_name, fs_name, None)
            .unwrap();

        let devnode = pool.get_filesystem_by_uuid(fs_uuid).unwrap().1.devnode();
        // Braces to ensure f is closed before destroy
        {
            let mut f = OpenOptions::new().write(true).open(devnode).unwrap();
            // Write 1 more sector than is initially allocated to a pool
            let write_size = datablocks_to_sectors(INITIAL_DATA_SIZE) + Sectors(1);
            let buf = &[1u8; SECTOR_SIZE];
            for i in 0..*write_size {
                f.write_all(buf).unwrap();
                // Simulate handling a DM event by extending the pool when
                // the amount of free space in pool has decreased to the
                // DATA_LOWATER value.
                if i == *(datablocks_to_sectors(INITIAL_DATA_SIZE - DATA_LOWATER)) {
                    pool.extend_thin_data_device(
                        pool_uuid,
                        &mut backstore,
                        datablocks_to_sectors(INITIAL_DATA_SIZE),
                    ).unwrap();
                }
            }
        }
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
        devlinks::setup_devlinks(Vec::new().into_iter()).unwrap();
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).unwrap();
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
        devlinks::pool_added(&pool_name).unwrap();
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
        devlinks::setup_devlinks(Vec::new().into_iter()).unwrap();
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
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
        devlinks::setup_devlinks(Vec::new().into_iter()).unwrap();
        let mut backstore =
            Backstore::initialize(pool_uuid, paths2, MIN_MDA_SECTORS, false).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
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
        backstore
            .add_blockdevs(pool_uuid, paths1, BlockDevTier::Cache, false)
            .unwrap();
        let new_device = backstore
            .device()
            .expect("Space already allocated from backstore, backstore must have device");
        assert!(old_device != new_device);
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

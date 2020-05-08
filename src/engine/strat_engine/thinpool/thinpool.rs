// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle management of a pool's thinpool device.

use std::{
    cmp::{max, min},
    fmt,
    thread::sleep,
    time::Duration,
};

use serde_json::Value;
use uuid::Uuid;

use devicemapper::{
    device_exists, DataBlocks, Device, DmDevice, DmName, DmNameBuf, FlakeyTargetParams, LinearDev,
    LinearDevTargetParams, LinearTargetParams, MetaBlocks, Sectors, TargetLine, ThinDevId,
    ThinPoolDev, ThinPoolStatus, ThinPoolStatusSummary, IEC,
};

use crate::{
    engine::{
        engine::Filesystem,
        event::{get_engine_listener_list, EngineEvent},
        strat_engine::{
            backstore::Backstore,
            cmd::{thin_check, thin_repair, udev_settle},
            device::wipe_sectors,
            devlinks,
            dm::get_dm,
            names::{
                format_flex_ids, format_thin_ids, format_thinpool_ids, FlexRole, ThinPoolRole,
                ThinRole,
            },
            serde_structs::{FlexDevsSave, Recordable, ThinPoolDevSave},
            thinpool::{
                filesystem::{FilesystemStatus, StratFilesystem},
                mdv::MetadataVol,
                thinids::ThinDevIdPool,
            },
        },
        structures::Table,
        types::{FilesystemUuid, MaybeDbusPath, Name, PoolUuid},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

pub const DATA_BLOCK_SIZE: Sectors = Sectors(2 * IEC::Ki);
pub const DATA_LOWATER: DataBlocks = DataBlocks(2048); // 2 GiB

const INITIAL_META_SIZE: MetaBlocks = MetaBlocks(4 * IEC::Ki);
// The smallest amount allocated to the thinpool meta device at one time
const MIN_META_SEGMENT_SIZE: MetaBlocks = MetaBlocks(4 * IEC::Ki);
const INITIAL_DATA_SIZE: DataBlocks = DataBlocks(768);
const INITIAL_MDV_SIZE: Sectors = Sectors(32 * IEC::Ki); // 16 MiB

const SPACE_CRIT_PCT: u8 = 95;

fn sectors_to_datablocks(sectors: Sectors) -> DataBlocks {
    DataBlocks(sectors / DATA_BLOCK_SIZE)
}

fn datablocks_to_sectors(data_blocks: DataBlocks) -> Sectors {
    *data_blocks * DATA_BLOCK_SIZE
}

// Return all the useful identifying information for a particular thinpool
// device mapper device that is available.
fn thin_pool_identifiers(thin_pool: &ThinPoolDev) -> String {
    format!(
        "devicemapper name: {}, device number: {}, device node: {}",
        thin_pool.name(),
        thin_pool.device(),
        thin_pool.devnode().display()
    )
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
/// Lowater needed for three things:
/// 1. Extend data device (currently not applicable due to greedy allocation)
/// 2. Get an event when pool exceeds SPACE_CRIT_PCT
/// 3. Get an event when usage has increased enough that we might need to
///    extend a filesystem
fn calc_lowater(used: DataBlocks, data_dev_size: DataBlocks, available: DataBlocks) -> DataBlocks {
    let total = data_dev_size + available;

    // Calculate #2. Calculated against total size.
    let crit_percent_total = (total * SPACE_CRIT_PCT) / 100u8;
    assert!(crit_percent_total < total);
    let low_water_for_crit = total - crit_percent_total;
    assert!(DataBlocks(std::u64::MAX) - available >= DATA_LOWATER);

    // Compare values of #1 and #2 above to get which one is higher
    // WARNING: Do not alter this if-expression to a max-expression.
    // Doing so would invalidate the assertion below.
    // Need to add available to LOWATER to make it apples-to-apples with
    // low_water_for_crit.
    let prelim_max = if DATA_LOWATER + available > low_water_for_crit {
        DATA_LOWATER
    } else {
        assert!(low_water_for_crit >= available);
        // Adjust for against end of data dev instead of total
        low_water_for_crit - available
    };

    // Calculate #3. This is not the same as #1 because pool might be fully
    // extended, but we still need events to extend filesystems
    let fs_event_lowater = DataBlocks((*(data_dev_size - used)).saturating_sub(*DATA_LOWATER));

    // Get the highest of the three values
    max(prelim_max, fs_event_lowater)
}

/// Segment lists that the ThinPool keeps track of.
#[derive(Debug)]
struct Segments {
    meta_segments: Vec<(Sectors, Sectors)>,
    meta_spare_segments: Vec<(Sectors, Sectors)>,
    data_segments: Vec<(Sectors, Sectors)>,
    mdv_segments: Vec<(Sectors, Sectors)>,
}

/// A way of digesting the status reported on the thinpool into a value
/// that can be checked for equality. This way, two statuses,
/// collected at different times can be checked to determine whether their
/// gross, as opposed to fine, differences are significant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThinPoolStatusDigest {
    Fail,
    Error,
    Good,
    ReadOnly,
    OutOfSpace,
}

impl From<&ThinPoolStatus> for ThinPoolStatusDigest {
    fn from(status: &ThinPoolStatus) -> ThinPoolStatusDigest {
        match status {
            ThinPoolStatus::Working(status) => match status.summary {
                ThinPoolStatusSummary::Good => ThinPoolStatusDigest::Good,
                ThinPoolStatusSummary::ReadOnly => ThinPoolStatusDigest::ReadOnly,
                ThinPoolStatusSummary::OutOfSpace => ThinPoolStatusDigest::OutOfSpace,
            },
            ThinPoolStatus::Fail => ThinPoolStatusDigest::Fail,
            ThinPoolStatus::Error => ThinPoolStatusDigest::Error,
        }
    }
}

/// In this implementation convert the status designations to strings which
/// match those strings that the kernel uses to identify the different states
/// in the ioctl result.
impl fmt::Display for ThinPoolStatusDigest {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ThinPoolStatusDigest::Good => write!(f, "rw"),
            ThinPoolStatusDigest::ReadOnly => write!(f, "ro"),
            ThinPoolStatusDigest::OutOfSpace => write!(f, "out_of_data_space"),
            ThinPoolStatusDigest::Fail => write!(f, "Fail"),
            ThinPoolStatusDigest::Error => write!(f, "Error"),
        }
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
    segments: Segments,
    id_gen: ThinDevIdPool,
    filesystems: Table<StratFilesystem>,
    mdv: MetadataVol,
    /// The single DM device that the backstore presents as its upper-most
    /// layer. All DM components obtain their storage from this layer.
    /// The device will change if the backstore adds or removes a cache.
    backstore_device: Device,
    thin_pool_status: Option<ThinPoolStatus>,
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

        let data_dev_size = data_dev.size();
        let thinpool_dev = ThinPoolDev::new(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            meta_dev,
            data_dev,
            data_block_size,
            calc_lowater(
                DataBlocks(0),
                sectors_to_datablocks(data_dev_size),
                sectors_to_datablocks(backstore.available_in_backstore()),
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
            thin_pool_status: None,
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

        let data_dev_size = data_dev.size();
        let thinpool_dev = ThinPoolDev::setup(
            get_dm(),
            &thinpool_name,
            Some(&thinpool_uuid),
            meta_dev,
            data_dev,
            thin_pool_save.data_block_size,
            calc_lowater(
                // This is smaller than the actual amount used. This value
                // is updated when the thinpool's check method is invoked.
                DataBlocks(0),
                sectors_to_datablocks(data_dev_size),
                sectors_to_datablocks(backstore.available_in_backstore()),
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
            thin_pool_status: None,
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
        assert_eq!(
            backstore.device().expect(
                "thinpool exists and has been allocated to, so backstore must have a cap device"
            ),
            self.backstore_device
        );

        let mut should_save: bool = false;
        let thin_pool_status = self.thin_pool.status(get_dm())?;

        if let ThinPoolStatus::Working(status) = &thin_pool_status {
            let usage = &status.usage;

            // Ensure meta subdevice is approx. 1/1000th of total usable
            // size
            let target_meta_size = (backstore.datatier_usable_size() / 1000u16).metablocks();
            if usage.total_meta < target_meta_size {
                let meta_request = target_meta_size - usage.total_meta;

                if meta_request > MIN_META_SEGMENT_SIZE {
                    should_save |= match self.extend_thin_meta_device(
                        pool_uuid,
                        backstore,
                        meta_request.sectors(),
                    ) {
                        Ok(extend_size) => extend_size != Sectors(0),
                        Err(_) => false,
                    };
                }
            }

            // Expand data blocks to fill all available remaining space
            let free_space = backstore.available_in_backstore();
            let total_extended = if free_space < DATA_BLOCK_SIZE {
                DataBlocks(0)
            } else {
                let amount_allocated =
                    match self.extend_thin_data_device(pool_uuid, backstore, free_space) {
                        Ok(extend_size) => extend_size,
                        Err(_) => Sectors(0),
                    };
                should_save |= amount_allocated != Sectors(0);
                sectors_to_datablocks(amount_allocated)
            };

            let current_total = usage.total_data + total_extended;

            let lowater = calc_lowater(
                usage.used_data,
                current_total,
                sectors_to_datablocks(backstore.available_in_backstore()),
            );

            self.thin_pool.set_low_water_mark(get_dm(), lowater)?;
            self.resume()?;
        }

        self.set_state(thin_pool_status);

        for (name, uuid, fs) in self.filesystems.iter_mut() {
            let (fs_status, save_mdv) = fs.check()?;
            if save_mdv {
                if let Err(e) = self.mdv.save_fs(name, *uuid, fs) {
                    error!("Could not save MDV for fs with UUID {} and name {} belonging to pool with UUID {}, reason: {:?}",
                                uuid, name, pool_uuid, e);
                }
            }
            if let FilesystemStatus::Failed = fs_status {
                // TODO: filesystem failed, how to recover?
            }
        }
        Ok(should_save)
    }

    /// Set the current status of the thin_pool device to thin_pool_status.
    /// If there has been a change, log that change at the info or warn level
    /// as appropriate.
    fn set_state(&mut self, thin_pool_status: ThinPoolStatus) {
        let current_status: Option<ThinPoolStatusDigest> =
            self.thin_pool_status.as_ref().map(|x| x.into());
        let new_status: ThinPoolStatusDigest = (&thin_pool_status).into();

        if current_status != Some(new_status) {
            let current_status_str = current_status
                .map(|x| x.to_string())
                .unwrap_or_else(|| "none".to_string());

            if new_status != ThinPoolStatusDigest::Good {
                warn!(
                    "Status of thinpool device with \"{}\" changed from \"{}\" to \"{}\"",
                    thin_pool_identifiers(&self.thin_pool),
                    current_status_str,
                    new_status,
                );
            } else {
                info!(
                    "Status of thinpool device with \"{}\" changed from \"{}\" to \"{}\"",
                    thin_pool_identifiers(&self.thin_pool),
                    current_status_str,
                    new_status,
                );
            }
        }

        self.thin_pool_status = Some(thin_pool_status);
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
            MIN_META_SEGMENT_SIZE.sectors(),
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
        assert!(modulus != Sectors(0));
        let sub_device_str = if data { "data" } else { "metadata" };
        let pool_uuid_str = pool_uuid.to_simple_ref();
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
                // (extend_size / modulus) * modulus is the maximum size that
                // Backstore::request() can return when operating correctly.
                if (extend_size / modulus) * modulus == actual_extend_size {
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

    /// The number of physical sectors in use by this thinpool abstraction.
    /// All sectors allocated to the mdv, all sectors allocated to the
    /// metadata spare, and all sectors actually in use by the thinpool DM
    /// device, either for the metadata device or for the data device.
    pub fn total_physical_used(&self) -> StratisResult<Sectors> {
        let (data_dev_used, meta_dev_used) = match &self.thin_pool_status {
            None => {
                let err_msg = format!(
                    "Unknown status for thin pool device with \"{}\"",
                    thin_pool_identifiers(&self.thin_pool)
                );
                return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
            }
            Some(status) => match status {
                ThinPoolStatus::Working(status) => (
                    datablocks_to_sectors(status.usage.used_data),
                    status.usage.used_meta.sectors(),
                ),
                ThinPoolStatus::Error => {
                    let err_msg = format!(
                        "Devicemapper could not obtain status for devicemapper thin pool device with \"{}\"",
                        thin_pool_identifiers(&self.thin_pool)
                    );
                    return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
                }
                ThinPoolStatus::Fail => {
                    let err_msg = format!(
                        "The thinpool device with \"{}\" has failed",
                        thin_pool_identifiers(&self.thin_pool)
                    );
                    return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
                }
            },
        };

        let spare_total = self.segments.meta_spare_segments.iter().map(|s| s.1).sum();

        let mdv_total = self.segments.mdv_segments.iter().map(|s| s.1).sum();

        Ok(data_dev_used + spare_total + meta_dev_used + mdv_total)
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

    pub fn filesystems(&self) -> Vec<(Name, FilesystemUuid, &dyn Filesystem)> {
        self.filesystems
            .iter()
            .map(|(name, uuid, x)| (name.clone(), *uuid, x as &dyn Filesystem))
            .collect()
    }

    pub fn filesystems_mut(&mut self) -> Vec<(Name, FilesystemUuid, &mut dyn Filesystem)> {
        self.filesystems
            .iter_mut()
            .map(|(name, uuid, x)| (name.clone(), *uuid, x as &mut dyn Filesystem))
            .collect()
    }

    /// Create a filesystem within the thin pool. Given name must not
    /// already be in use.
    pub fn create_filesystem(
        &mut self,
        pool_uuid: PoolUuid,
        _pool_name: &str,
        name: &str,
        size: Option<Sectors>,
    ) -> StratisResult<FilesystemUuid> {
        let (fs_uuid, mut new_filesystem) =
            StratFilesystem::initialize(pool_uuid, &self.thin_pool, size, self.id_gen.new_id()?)?;
        let name = Name::new(name.to_owned());
        if let Err(err) = self.mdv.save_fs(&name, fs_uuid, &new_filesystem) {
            udev_settle().unwrap_or_else(|err| {
                warn!("{}", err);
                sleep(Duration::from_secs(5));
            });
            if let Err(err2) = new_filesystem.destroy(&self.thin_pool) {
                error!(
                    "When handling failed save_fs(), fs.destroy() failed: {}",
                    err2
                )
            }
            return Err(err);
        }
        #[cfg(test)]
        devlinks::filesystem_added(_pool_name, &name, &new_filesystem.devnode());
        self.filesystems.insert(name, fs_uuid, new_filesystem);

        Ok(fs_uuid)
    }

    /// Create a filesystem snapshot of the origin.  Given origin_uuid
    /// must exist.  Returns the Uuid of the new filesystem.
    pub fn snapshot_filesystem(
        &mut self,
        pool_uuid: PoolUuid,
        _pool_name: &str,
        origin_uuid: FilesystemUuid,
        snapshot_name: &str,
    ) -> StratisResult<(FilesystemUuid, &mut dyn Filesystem)> {
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
        #[cfg(test)]
        devlinks::filesystem_added(_pool_name, &new_fs_name, &new_filesystem.devnode());
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
    ///
    /// * Ok(Some(uuid)) provides the uuid of the destroyed filesystem
    /// * Ok(None) is returned if the filesystem did not exist
    /// * Err(_) is returned if the filesystem could not be destroyed
    pub fn destroy_filesystem(
        &mut self,
        pool_name: &str,
        uuid: FilesystemUuid,
    ) -> StratisResult<Option<FilesystemUuid>> {
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
                    #[cfg(test)]
                    devlinks::filesystem_removed(pool_name, &fs_name);
                    Ok(Some(uuid))
                }
                Err(err) => {
                    self.filesystems.insert(fs_name, uuid, fs);
                    Err(err)
                }
            },
            None => Ok(None),
        }
    }

    #[cfg(test)]
    pub fn state(&self) -> Option<&ThinPoolStatus> {
        self.thin_pool_status.as_ref()
    }

    /// Rename a filesystem within the thin pool.
    ///
    /// * Ok(Some(uuid)) provides the uuid of the renamed filesystem
    /// * Ok(None) is returned if the source and target filesystem names are the same
    /// * Err(StratisError::Engine(ErrorEnum::NotFound, _)) is returned if the source
    /// filesystem name does not exist
    /// * Err(StratisError::Engine(ErrorEnum::AlreadyExists, _)) is returned if the target
    /// filesystem name already exists
    pub fn rename_filesystem(
        &mut self,
        pool_name: &str,
        uuid: FilesystemUuid,
        new_name: &str,
    ) -> StratisResult<Option<Uuid>> {
        let old_name = rename_filesystem_pre!(self; uuid; new_name);
        let new_name = Name::new(new_name.to_owned());

        let filesystem = self
            .filesystems
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
            Ok(Some(uuid))
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
        // If MDV suspend fails, resume the thin pool and return the error
        if let Err(err) = self.mdv.suspend() {
            self.thin_pool.resume(get_dm()).map_err(|e| {
                StratisError::Error(format!(
                    "Suspending the MDV failed: {}. MDV suspend clean up action \
                     of resuming the thin pool also failed: {}.",
                    err, e
                ))
            })?;
            Err(err)
        } else {
            Ok(())
        }
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

        let meta_table = self
            .thin_pool
            .meta_dev()
            .table()
            .table
            .clone()
            .iter()
            .map(&xform_target_line)
            .collect::<Vec<_>>();

        let data_table = self
            .thin_pool
            .data_dev()
            .table()
            .table
            .clone()
            .iter()
            .map(&xform_target_line)
            .collect::<Vec<_>>();

        let mdv_table = self
            .mdv
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

    pub fn set_dbus_path(&mut self, path: MaybeDbusPath) {
        self.dbus_path = path
    }
}

impl<'a> Into<Value> for &'a ThinPool {
    fn into(self) -> Value {
        json!({
            "filesystems": Value::Array(
                self.filesystems.iter()
                    .map(|(name, uuid, _)| json!({
                        "name": name.to_string(),
                        "uuid": uuid.to_simple_ref().to_string(),
                    }))
                    .collect()
            )
        })
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
#[allow(clippy::type_complexity)]
fn setup_metadev(
    pool_uuid: PoolUuid,
    thinpool_name: &DmName,
    device: Device,
    meta_segments: Vec<(Sectors, Sectors)>,
    spare_segments: Vec<(Sectors, Sectors)>,
) -> StratisResult<(LinearDev, Vec<(Sectors, Sectors)>, Vec<(Sectors, Sectors)>)> {
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
    use std::{
        fs::OpenOptions,
        io::{BufWriter, Read, Write},
        path::Path,
    };

    use nix::mount::{mount, umount, MsFlags};
    use uuid::Uuid;

    use devicemapper::{Bytes, SECTOR_SIZE};

    use crate::engine::strat_engine::{
        backstore::MDADataSize,
        device::SyncAll,
        tests::{loopbacked, real},
    };

    use crate::engine::strat_engine::thinpool::filesystem::{fs_usage, FILESYSTEM_LOWATER};

    use super::*;

    const BYTES_PER_WRITE: usize = 2 * IEC::Ki as usize * SECTOR_SIZE as usize;

    /// Test greedy allocation.
    /// Verify that ThinPool::new() allocates nearly everything available.
    /// Verify that meta and data devices are roughly in their correct
    /// proportion.
    /// FIXME: This is a temporary test; it should be removed when greedy
    /// allocation is removed.
    fn test_greedy_allocation(paths: &[&Path]) {
        let pool_uuid = Uuid::new_v4();
        devlinks::setup_dev_path().unwrap();
        devlinks::cleanup_devlinks(Vec::new().into_iter());

        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MDADataSize::default(), None).unwrap();

        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        )
        .unwrap();

        pool.check(pool_uuid, &mut backstore).unwrap();

        assert!(backstore.available_in_backstore() < DATA_BLOCK_SIZE);

        let meta_size = pool.thin_pool.meta_dev().size();
        let data_size = pool.thin_pool.data_dev().size();
        assert!(meta_size * 1500u64 > data_size);
        assert!(meta_size * 500u64 < data_size || meta_size.metablocks() == INITIAL_META_SIZE);
    }

    #[test]
    fn loop_test_greedy_allocation() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_greedy_allocation,
        );
    }

    #[test]
    fn real_test_greedy_allocation() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_greedy_allocation,
        );
    }

    /// Verify that a full pool extends properly when additional space is added.
    fn test_full_pool(paths: &[&Path]) {
        let pool_uuid = Uuid::new_v4();
        devlinks::setup_dev_path().unwrap();
        devlinks::cleanup_devlinks(Vec::new().into_iter());
        let (first_path, remaining_paths) = paths.split_at(1);
        let mut backstore =
            Backstore::initialize(pool_uuid, first_path, MDADataSize::default(), None).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        )
        .unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(pool_name);
        let fs_uuid = pool
            .create_filesystem(pool_uuid, pool_name, "stratis_test_filesystem", None)
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
            )
            .unwrap();
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
                    ThinPoolStatus::Error => panic!("Could not obtain status for thinpool."),
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
            ThinPoolStatus::Error => panic!("Could not obtain status for thinpool."),
            ThinPoolStatus::Fail => panic!("ThinPoolStatus::Fail  Expected working/full."),
        };

        // Add block devices to the pool and run check() to extend
        backstore.add_datadevs(pool_uuid, remaining_paths).unwrap();
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
            ThinPoolStatus::Error => panic!("Could not obtain status for thinpool."),
            ThinPoolStatus::Fail => panic!("ThinPoolStatus::Fail.  Expected working/good."),
        };
    }

    #[test]
    fn loop_test_full_pool() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(2, Some(Bytes(IEC::Gi).sectors())),
            test_full_pool,
        );
    }

    #[test]
    fn real_test_full_pool() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(
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
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MDADataSize::default(), None).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        )
        .unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(pool_name);
        let fs_uuid = pool
            .create_filesystem(pool_uuid, pool_name, "stratis_test_filesystem", None)
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
            )
            .unwrap();
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
        )
        .unwrap();

        let (_, snapshot_filesystem) = pool
            .snapshot_filesystem(pool_uuid, pool_name, fs_uuid, "test_snapshot")
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
            )
            .unwrap();
            for i in 0..file_count {
                let file_path = snapshot_tmp_dir
                    .path()
                    .join(format!("stratis_test{}.txt", i));
                let mut f = OpenOptions::new().read(true).open(file_path).unwrap();
                f.read_exact(&mut read_buf).unwrap();
                assert_eq!(read_buf[0..SECTOR_SIZE], write_buf[0..SECTOR_SIZE]);
            }
        }
    }

    #[test]
    fn loop_test_filesystem_snapshot() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_filesystem_snapshot,
        );
    }

    #[test]
    fn real_test_filesystem_snapshot() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
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
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MDADataSize::default(), None).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        )
        .unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(pool_name);
        let fs_uuid = pool
            .create_filesystem(pool_uuid, pool_name, name1, None)
            .unwrap();

        let action = pool.rename_filesystem(pool_name, fs_uuid, name2).unwrap();
        assert_matches!(action, Some(_));
        let flexdevs: FlexDevsSave = pool.record();
        let thinpoolsave: ThinPoolDevSave = pool.record();
        pool.teardown().unwrap();

        let pool = ThinPool::setup(pool_uuid, &thinpoolsave, &flexdevs, &backstore).unwrap();

        assert_eq!(&*pool.get_filesystem_by_uuid(fs_uuid).unwrap().0, name2);
    }

    #[test]
    fn loop_test_filesystem_rename() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_filesystem_rename,
        );
    }

    #[test]
    fn real_test_filesystem_rename() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(1, None, None),
            test_filesystem_rename,
        );
    }

    /// Verify that setting up a pool when the pool has not been previously torn
    /// down does not fail. Clutter the original pool with a filesystem with
    /// some data on it.
    fn test_pool_setup(paths: &[&Path]) {
        let pool_uuid = Uuid::new_v4();
        devlinks::cleanup_devlinks(Vec::new().into_iter());
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MDADataSize::default(), None).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        )
        .unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(pool_name);
        let fs_uuid = pool
            .create_filesystem(pool_uuid, pool_name, "fsname", None)
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
            )
            .unwrap();
            writeln!(
                &OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(new_file)
                    .unwrap(),
                "data"
            )
            .unwrap();
        }
        let thinpooldevsave: ThinPoolDevSave = pool.record();

        let new_pool =
            ThinPool::setup(pool_uuid, &thinpooldevsave, &pool.record(), &backstore).unwrap();

        assert!(new_pool.get_filesystem_by_uuid(fs_uuid).is_some());
    }

    #[test]
    fn loop_test_pool_setup() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_pool_setup,
        );
    }

    #[test]
    fn real_test_pool_setup() {
        real::test_with_spec(&real::DeviceLimits::AtLeast(1, None, None), test_pool_setup);
    }
    /// Verify that destroy_filesystems actually deallocates the space
    /// from the thinpool, by attempting to reinstantiate it using the
    /// same thin id and verifying that it fails.
    fn test_thindev_destroy(paths: &[&Path]) {
        let pool_uuid = Uuid::new_v4();
        devlinks::cleanup_devlinks(Vec::new().into_iter());
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MDADataSize::default(), None).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        )
        .unwrap();
        let pool_name = "stratis_test_pool";
        devlinks::pool_added(pool_name);
        let fs_name = "stratis_test_filesystem";
        let fs_uuid = pool
            .create_filesystem(pool_uuid, pool_name, fs_name, None)
            .unwrap();
        pool.destroy_filesystem(pool_name, fs_uuid).unwrap();
        let flexdevs: FlexDevsSave = pool.record();
        let thinpooldevsave: ThinPoolDevSave = pool.record();
        pool.teardown().unwrap();

        // Check that destroyed fs is not present in MDV. If the record
        // had been left on the MDV that didn't match a thin_id in the
        // thinpool, ::setup() will fail.
        let pool = ThinPool::setup(pool_uuid, &thinpooldevsave, &flexdevs, &backstore).unwrap();

        assert_matches!(pool.get_filesystem_by_uuid(fs_uuid), None);
    }

    #[test]
    fn loop_test_thindev_destroy() {
        // This test requires more than 1 GiB.
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_thindev_destroy,
        );
    }

    #[test]
    fn real_test_thindev_destroy() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(1, None, None),
            test_thindev_destroy,
        );
    }

    /// Verify that the logical space allocated to a filesystem is expanded when
    /// the number of sectors written to the filesystem causes the free space to
    /// dip below the FILESYSTEM_LOWATER mark. Verify that the filesystem space has
    /// been expanded by calling pool.check() then looking at the total space used
    /// compared to the original size.  Verify that the MDV has been updated
    /// by tearing down/reconstructing the pool and verify the thindev size is updated.
    fn test_thindev_expand(paths: &[&Path]) {
        let start_thindev_size: Sectors;
        let pool_uuid = Uuid::new_v4();
        devlinks::cleanup_devlinks(Vec::new().into_iter());
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MDADataSize::default(), None).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        )
        .unwrap();

        // Create a filesystem as small as possible.  Allocate 1 MiB bigger than
        // the low water mark.
        let fs_size = FILESYSTEM_LOWATER + Bytes(IEC::Mi).sectors();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(pool_name);
        let fs_name = "stratis_test_filesystem";
        let fs_uuid = pool
            .create_filesystem(pool_uuid, pool_name, fs_name, Some(fs_size))
            .unwrap();
        let tmp_dir = tempfile::Builder::new()
            .prefix("stratis_testing")
            .tempdir()
            .unwrap();

        // Braces to ensure the mutable borrow of pool is limited
        {
            let filesystem = pool.get_mut_filesystem_by_uuid(fs_uuid).unwrap().1;
            start_thindev_size = filesystem.thindev_size();

            mount(
                Some(&filesystem.devnode()),
                tmp_dir.path(),
                Some("xfs"),
                MsFlags::empty(),
                None as Option<&str>,
            )
            .unwrap();
        }
        // Write 2 MiB of data. The filesystem's free space is now 1 MiB
        // below FILESYSTEM_LOWATER.
        let write_size = Bytes(IEC::Mi * 2).sectors();
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
        let (orig_fs_total_bytes, _) = fs_usage(tmp_dir.path()).unwrap();
        // Simulate handling a DM event by running a pool check.
        pool.check(pool_uuid, &mut backstore).unwrap();
        let (fs_total_bytes, _) = fs_usage(tmp_dir.path()).unwrap();
        assert!(fs_total_bytes > orig_fs_total_bytes);
        umount(tmp_dir.path()).unwrap();

        // Teardown and reconstruct the pool to verify the MDV has the updated size
        let flexdevs: FlexDevsSave = pool.record();
        let thinpoolsave: ThinPoolDevSave = pool.record();
        pool.teardown().unwrap();
        let mut pool = ThinPool::setup(pool_uuid, &thinpoolsave, &flexdevs, &backstore).unwrap();
        let filesystem = pool.get_mut_filesystem_by_uuid(fs_uuid).unwrap().1;
        let thindev_size = filesystem.thindev_size();
        assert!(thindev_size > start_thindev_size)
    }

    #[test]
    fn loop_test_thindev_expand() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_thindev_expand,
        );
    }

    #[test]
    fn real_test_thindev_expand() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(1, None, None),
            test_thindev_expand,
        );
    }

    /// Just suspend and resume the device and make sure it doesn't crash.
    /// Suspend twice in succession and then resume twice in succession
    /// to check idempotency.
    fn test_suspend_resume(paths: &[&Path]) {
        let pool_uuid = Uuid::new_v4();
        devlinks::cleanup_devlinks(Vec::new().into_iter());
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MDADataSize::default(), None).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        )
        .unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(pool_name);
        pool.create_filesystem(pool_uuid, pool_name, "stratis_test_filesystem", None)
            .unwrap();

        pool.suspend().unwrap();
        pool.suspend().unwrap();
        pool.resume().unwrap();
        pool.resume().unwrap();
    }

    #[test]
    fn loop_test_suspend_resume() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_suspend_resume,
        );
    }

    #[test]
    fn real_test_suspend_resume() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(1, None, None),
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
        let mut backstore =
            Backstore::initialize(pool_uuid, paths2, MDADataSize::default(), None).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        )
        .unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(pool_name);
        let fs_uuid = pool
            .create_filesystem(pool_uuid, pool_name, "stratis_test_filesystem", None)
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
            )
            .unwrap();
            OpenOptions::new()
                .create(true)
                .write(true)
                .open(&new_file)
                .unwrap()
                .write_all(bytestring)
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
        backstore.init_cache(pool_uuid, paths1).unwrap();
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
                .read_exact(&mut buf)
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
    fn loop_test_set_device() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_set_device,
        );
    }

    #[test]
    fn real_test_set_device() {
        real::test_with_spec(&real::DeviceLimits::AtLeast(2, None, None), test_set_device);
    }
}

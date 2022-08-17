// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle management of a pool's thinpool device.

use std::{
    cmp::{max, min},
    collections::HashMap,
    fmt,
};

use crossbeam::thread::scope;
use retry::{delay::Fixed, retry_with_index};
use serde_json::{Map, Value};

use devicemapper::{
    device_exists, Bytes, DataBlocks, Device, DmDevice, DmName, DmNameBuf, DmOptions,
    FlakeyTargetParams, LinearDev, LinearDevTargetParams, LinearTargetParams, MetaBlocks, Sectors,
    TargetLine, ThinDevId, ThinPoolDev, ThinPoolStatus, ThinPoolStatusSummary, ThinPoolUsage, IEC,
};

use crate::{
    engine::{
        engine::{DumpState, StateDiff},
        strat_engine::{
            backstore::Backstore,
            cmd::{thin_check, thin_metadata_size, thin_repair},
            dm::get_dm,
            names::{
                format_flex_ids, format_thin_ids, format_thinpool_ids, FlexRole, ThinPoolRole,
                ThinRole,
            },
            serde_structs::{FlexDevsSave, Recordable, ThinPoolDevSave},
            thinpool::{filesystem::StratFilesystem, mdv::MetadataVol, thinids::ThinDevIdPool},
            writing::wipe_sectors,
        },
        structures::Table,
        types::{Compare, FilesystemUuid, Name, PoolUuid, StratFilesystemDiff, ThinPoolDiff},
    },
    stratis::{StratisError, StratisResult},
};

// Maximum number of thin devices (filesystems) allowed on a thin pool.
// NOTE: This will eventually become a default configurable by the user.
const DEFAULT_FS_LIMIT: u64 = 100;

// 1 MiB
pub const DATA_BLOCK_SIZE: Sectors = Sectors(2 * IEC::Ki);

// 512 MiB
const INITIAL_MDV_SIZE: Sectors = Sectors(IEC::Mi);

// Use different constants for testing and application builds.
use self::consts::{DATA_ALLOC_SIZE, DATA_LOWATER};
#[cfg(not(test))]
mod consts {
    use super::{DataBlocks, IEC};

    // 50 GiB
    pub const DATA_ALLOC_SIZE: DataBlocks = DataBlocks(50 * IEC::Ki);
    // 15 GiB
    pub const DATA_LOWATER: DataBlocks = DataBlocks(15 * IEC::Ki);
}
#[cfg(test)]
mod consts {
    use super::{DataBlocks, IEC};

    // 5 GiB
    pub const DATA_ALLOC_SIZE: DataBlocks = DataBlocks(5 * IEC::Ki);
    // 4 GiB
    pub const DATA_LOWATER: DataBlocks = DataBlocks(4 * IEC::Ki);
}

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
pub enum ThinPoolStatusDigest {
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThinPoolStatusDigest::Good => write!(f, "rw"),
            ThinPoolStatusDigest::ReadOnly => write!(f, "ro"),
            ThinPoolStatusDigest::OutOfSpace => write!(f, "out_of_data_space"),
            ThinPoolStatusDigest::Fail => write!(f, "Fail"),
            ThinPoolStatusDigest::Error => write!(f, "Error"),
        }
    }
}

/// Returns the determined size of the data and metadata devices.
///
/// upper_limit is the upper bound for the data device size and will always
/// be too large to fit on the pool when combined with metadata size.
///
/// lower_limit is the lower bound for the data device and will always be small
/// enough to fit on the pool when combined with metadata size. Once the algorithm
/// converges, the lower limit is the data device size that yields optimal
/// space usage.
///
/// This method will always return optimal values that leave as little space as
/// possible free, thus optimizing storage usage. Because the data device size
/// must be a multiple of DATA_BLOCK_SIZE, there may be some free space left over
/// in the optimal case. The unused space should not be larger than
/// 2 * DATA_BLOCK_SIZE. Free space of size >= DATA_BLOCK_SIZE can occur
/// if DATA_BLOCK_SIZE space is left and increasing the the data device size by
/// DATA_BLOCK_SIZE results in an increase in the metadata device size as well.
/// This would cause current_size + DATA_BLOCK_SIZE to be used as the upper limit
/// instead of the lower limit as it would be too large to fit on the pool.
///
/// Because this method needs to make room for the spare metadata space as well,
/// you will see the metadata size multiplied by 2.
///
/// This method is recursive.
///
/// Precondition: upper_limit + 2 * thin_metadata_size(upper_limit) > total_space
/// Precondition: lower_limit + 2 * thin_metadata_size(upper_limit) <= total_space
fn search(
    total_space: Sectors,
    upper_limit: DataBlocks,
    lower_limit: DataBlocks,
    fs_limit: u64,
) -> StratisResult<(Sectors, Sectors)> {
    let diff = upper_limit - lower_limit;
    let half = lower_limit + diff / 2u64 + diff % 2u64;
    let half_meta = thin_metadata_size(DATA_BLOCK_SIZE, datablocks_to_sectors(half), fs_limit)?;

    if diff == DataBlocks(1) {
        let upper_limit_sectors = datablocks_to_sectors(upper_limit);
        let lower_limit_sectors = datablocks_to_sectors(lower_limit);
        let upper_meta = thin_metadata_size(DATA_BLOCK_SIZE, upper_limit_sectors, fs_limit)?;
        let lower_meta = thin_metadata_size(DATA_BLOCK_SIZE, lower_limit_sectors, fs_limit)?;
        trace!(
            "Upper data: {}, upper meta: {}, lower data: {}, lower meta: {}, total space: {}",
            upper_limit_sectors,
            upper_meta,
            lower_limit_sectors,
            lower_meta,
            total_space
        );
        assert!(upper_limit_sectors + upper_meta * 2u64 > total_space);
        assert!(lower_limit_sectors + lower_meta * 2u64 <= total_space);
        Ok((lower_limit_sectors, lower_meta))
    } else if datablocks_to_sectors(half) + half_meta * 2u64 > total_space {
        search(total_space, half, lower_limit, fs_limit)
    } else {
        search(total_space, upper_limit, half, fs_limit)
    }
}

/// This method divides the total space into optimized data and metadata size
/// extensions. It returns values from search() respresenting the new total size
/// of these devices.
///
/// available_space represents the amount of space on the thin pool that has not
/// yet been allocated.
fn divide_space(
    available_space: Sectors,
    current_data_size: Sectors,
    current_meta_size: Sectors,
    fs_limit: u64,
) -> StratisResult<(Sectors, Sectors)> {
    // The below expression of DataBlocks(1) is added because we must round up to the
    // next largest data block which will always be a valid upper limit.
    //
    // This is required to satisfy the preconditions of search.
    let upper_data_aligned =
        sectors_to_datablocks(current_data_size + available_space) + DataBlocks(1);
    let upper_limit_meta_size = thin_metadata_size(
        DATA_BLOCK_SIZE,
        datablocks_to_sectors(upper_data_aligned),
        fs_limit,
    )?;
    let total_space = current_data_size + 2u64 * current_meta_size + available_space;

    assert!(datablocks_to_sectors(upper_data_aligned) + upper_limit_meta_size * 2u64 > total_space);
    assert!(current_data_size % DATA_BLOCK_SIZE == Sectors(0));

    search(total_space, upper_data_aligned, DataBlocks(0), fs_limit)
}

/// Finds the optimized size for the data and metadata extension.
///
/// This method will take either the data allocation size or the full amount of space
/// left (if this is less than the data allocation size) and segment it into the
/// appropriate sizes to maximize the amount of data device extension possible while
/// still extending the metadata devices to have room for the new requirements.
///
/// This method returns the extension size, not the total size.
fn calculate_subdevice_extension(
    available_space: Sectors,
    current_data_size: Sectors,
    current_meta_size: Sectors,
    requested_space: Sectors,
    fs_limit: u64,
) -> StratisResult<(Sectors, Sectors)> {
    let requested_min = min(available_space, requested_space);

    debug!("Space for extension: {}", available_space);
    debug!("Current data device size: {}", current_data_size);
    debug!("Current meta device size: {}", current_meta_size);

    let (data, meta) = divide_space(
        requested_min,
        current_data_size,
        current_meta_size,
        fs_limit,
    )?;

    let data_extended = Sectors(data.saturating_sub(*current_data_size));
    debug!("Data extension: {}", data_extended);
    let meta_extended = Sectors(meta.saturating_sub(*current_meta_size));
    debug!("Meta extension: {}", meta_extended);

    assert!(available_space >= data_extended + 2u64 * meta_extended);
    assert_eq!(data_extended % DATA_BLOCK_SIZE, Sectors(0));

    if data_extended == Sectors(0) && meta_extended == Sectors(0) {
        Err(StratisError::OutOfSpaceError(format!(
            "{} requested but no space is available",
            requested_min
        )))
    } else {
        Ok((data_extended, meta_extended))
    }
}

pub struct ThinPoolSizeParams {
    meta_size: MetaBlocks,
    data_size: DataBlocks,
    mdv_size: Sectors,
}

impl ThinPoolSizeParams {
    /// Create a new set of initial sizes for all flex devices.
    pub fn new(available_space: Sectors) -> StratisResult<Self> {
        let initial_space = min(
            Sectors(available_space.saturating_sub(*INITIAL_MDV_SIZE)),
            datablocks_to_sectors(DATA_ALLOC_SIZE),
        );

        let (data_size, meta_size) =
            divide_space(initial_space, Sectors(0), Sectors(0), DEFAULT_FS_LIMIT)?;

        Ok(ThinPoolSizeParams {
            data_size: sectors_to_datablocks(data_size),
            meta_size: meta_size.metablocks(),
            mdv_size: INITIAL_MDV_SIZE,
        })
    }

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

/// Meta info type for metadata and spare metadata areas.
type MetaInfo<'a> = (
    Sectors,
    &'a mut Vec<(Sectors, Sectors)>,
    &'a mut Vec<(Sectors, Sectors)>,
);

/// Convert the thin pool status to usage information.
fn status_to_usage(status: Option<ThinPoolStatus>) -> Option<ThinPoolUsage> {
    status.and_then(|s| {
        if let ThinPoolStatus::Working(w) = s {
            Some(w.usage)
        } else {
            None
        }
    })
}

/// The number of physical sectors in use by this thinpool abstraction.
/// All sectors allocated to the mdv, all sectors allocated to the
/// metadata spare, and all sectors actually in use by the thinpool DM
/// device, either for the metadata device or for the data device.
fn calc_total_physical_used(data_used: Option<Sectors>, segments: &Segments) -> Option<Sectors> {
    let data_dev_used = data_used?;

    let meta_total = segments.meta_segments.iter().map(|s| s.1).sum();
    let spare_total = segments.meta_spare_segments.iter().map(|s| s.1).sum();

    let mdv_total = segments.mdv_segments.iter().map(|s| s.1).sum();

    Some(data_dev_used + spare_total + meta_total + mdv_total)
}

/// A ThinPool struct contains the thinpool itself, the spare
/// segments for its metadata device, and the filesystems and filesystem
/// metadata associated with it.
#[derive(Debug)]
pub struct ThinPool {
    thin_pool: ThinPoolDev,
    segments: Segments,
    id_gen: ThinDevIdPool,
    filesystems: Table<FilesystemUuid, StratFilesystem>,
    mdv: MetadataVol,
    /// The single DM device that the backstore presents as its upper-most
    /// layer. All DM components obtain their storage from this layer.
    /// The device will change if the backstore adds or removes a cache.
    backstore_device: Device,
    thin_pool_status: Option<ThinPoolStatusDigest>,
    allocated_size: Sectors,
    used: Option<ThinPoolUsage>,
    fs_limit: u64,
    enable_overprov: bool,
}

impl ThinPool {
    /// Make a new thin pool.
    pub fn new(
        pool_uuid: PoolUuid,
        thin_pool_size: &ThinPoolSizeParams,
        data_block_size: Sectors,
        backstore: &mut Backstore,
    ) -> StratisResult<ThinPool> {
        let mut segments_list = match backstore.request_alloc(&[
            thin_pool_size.meta_size(),
            thin_pool_size.meta_size(),
            thin_pool_size.data_size(),
            thin_pool_size.mdv_size(),
        ])? {
            Some(trans) => {
                let segs = trans.get_backstore();
                backstore.commit_alloc(pool_uuid, trans)?;
                segs
            }
            None => {
                let err_msg = "Could not allocate sufficient space for thinpool devices.";
                return Err(StratisError::Msg(err_msg.into()));
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
            // Either set the low water mark to the standard low water mark if
            // the device is larger than DATA_LOWATER or otherwise to half of the
            // capacity of the data device.
            min(
                DATA_LOWATER,
                DataBlocks((data_dev_size / DATA_BLOCK_SIZE) / 2),
            ),
            vec![
                "no_discard_passdown".to_string(),
                "skip_block_zeroing".to_string(),
            ],
        )?;

        let thin_pool_status = thinpool_dev.status(get_dm(), DmOptions::default()).ok();
        let digest = thin_pool_status.as_ref().map(|s| s.into());
        let segments = Segments {
            meta_segments: vec![meta_segments],
            meta_spare_segments: vec![spare_segments],
            data_segments: vec![data_segments],
            mdv_segments: vec![mdv_segments],
        };
        Ok(ThinPool {
            thin_pool: thinpool_dev,
            segments,
            id_gen: ThinDevIdPool::new_from_ids(&[]),
            filesystems: Table::default(),
            mdv,
            backstore_device,
            thin_pool_status: digest,
            allocated_size: backstore.datatier_allocated_size(),
            used: status_to_usage(thin_pool_status),
            fs_limit: DEFAULT_FS_LIMIT,
            enable_overprov: true,
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
        pool_name: &str,
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

        // TODO: Remove in stratisd 4.0.
        let mut migrate = false;

        let data_dev_size = data_dev.size();
        let mut thinpool_dev = ThinPoolDev::setup(
            get_dm(),
            &thinpool_name,
            Some(&thinpool_uuid),
            meta_dev,
            data_dev,
            thin_pool_save.data_block_size,
            // This is a larger amount of free space than the actual amount of free
            // space currently which will cause the value to be updated when the
            // thinpool's check method is invoked.
            sectors_to_datablocks(data_dev_size),
            thin_pool_save
                .feature_args
                .as_ref()
                .map(|hs| hs.iter().cloned().collect::<Vec<_>>())
                .unwrap_or_else(|| {
                    migrate = true;
                    vec![
                        "no_discard_passdown".to_owned(),
                        "skip_block_zeroing".to_owned(),
                        "error_if_no_space".to_owned(),
                    ]
                }),
        )?;

        // TODO: Remove in stratisd 4.0.
        if migrate {
            thinpool_dev.queue_if_no_space(get_dm())?;
        }

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
                    Ok(fs) => {
                        fs.udev_fs_change(pool_name, fssave.uuid, &fssave.name);
                        Some((Name::new(fssave.name.to_owned()), fssave.uuid, fs))
                    },
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
                return Err(StratisError::Msg(err_msg.into()));
            }
        }

        let thin_ids: Vec<ThinDevId> = filesystem_metadatas.iter().map(|x| x.thin_id).collect();
        let thin_pool_status = thinpool_dev.status(get_dm(), DmOptions::default()).ok();
        let digest = thin_pool_status.as_ref().map(|s| s.into());
        let segments = Segments {
            meta_segments,
            meta_spare_segments: spare_segments,
            data_segments,
            mdv_segments,
        };

        let fs_limit = thin_pool_save.fs_limit.unwrap_or_else(|| {
            max(fs_table.len(), convert_const!(DEFAULT_FS_LIMIT, u64, usize)) as u64
        });

        Ok(ThinPool {
            thin_pool: thinpool_dev,
            segments,
            id_gen: ThinDevIdPool::new_from_ids(&thin_ids),
            filesystems: fs_table,
            mdv,
            backstore_device,
            thin_pool_status: digest,
            allocated_size: backstore.datatier_allocated_size(),
            used: status_to_usage(thin_pool_status),
            fs_limit,
            enable_overprov: thin_pool_save.enable_overprov.unwrap_or(true),
        })
    }

    /// Get the last cached value for the total amount of space used on the pool.
    /// Stratis metadata size will be added a layer about my StratPool.
    pub fn total_physical_used(&self) -> Option<Sectors> {
        calc_total_physical_used(self.data_used(), &self.segments)
    }

    /// Get the last cached value for the total amount of data space used on the
    /// thin pool.
    fn data_used(&self) -> Option<Sectors> {
        self.used
            .as_ref()
            .map(|u| datablocks_to_sectors(u.used_data))
    }

    /// Run status checks and take actions on the thinpool and its components.
    /// The boolean in the return value indicates if a configuration change requiring a
    /// metadata save has been made.
    pub fn check(
        &mut self,
        pool_uuid: PoolUuid,
        backstore: &mut Backstore,
    ) -> StratisResult<(bool, ThinPoolDiff)> {
        assert_eq!(
            backstore.device().expect(
                "thinpool exists and has been allocated to, so backstore must have a cap device"
            ),
            self.backstore_device
        );

        let mut should_save: bool = false;

        let old_state = self.cached();

        if let Some(usage) = self.data_used() {
            if self.thin_pool.data_dev().size() - usage < datablocks_to_sectors(DATA_LOWATER)
                && !self.out_of_alloc_space()
            {
                let amount_allocated = match self.extend_thin_data_device(pool_uuid, backstore) {
                    (changed, Ok(extend_size)) => {
                        should_save |= changed;
                        extend_size
                    }
                    (changed, Err(e)) => {
                        should_save |= changed;
                        warn!("Device extension failed: {}", e);
                        (Sectors(0), Sectors(0))
                    }
                };
                should_save |= amount_allocated.0 != Sectors(0) || amount_allocated.1 != Sectors(0);

                self.thin_pool.set_low_water_mark(get_dm(), DATA_LOWATER)?;
                self.resume()?;
            }
        }

        let new_state = self.dump(backstore);

        Ok((should_save, old_state.diff(&new_state)))
    }

    /// Sum the logical size of all filesystems on the pool.
    fn filesystem_logical_size_sum(&self) -> Sectors {
        self.filesystems
            .iter()
            .map(|(_, _, fs)| fs.thindev_size())
            .sum()
    }

    /// Check all filesystems on this thin pool and return which had their sizes
    /// extended, if any. This method should not need to handle thin pool status
    /// because it never alters the thin pool itself.
    pub fn check_fs(
        &mut self,
        pool_uuid: PoolUuid,
        backstore: &Backstore,
    ) -> StratisResult<HashMap<FilesystemUuid, StratFilesystemDiff>> {
        let mut updated = HashMap::default();
        let data_size = self.thin_pool.data_dev().size();
        let min_data_size = self.min_datadev_size(backstore)?;
        let mut remaining_space = if !self.enable_overprov {
            let data_space = if self.out_of_alloc_space() || data_size > min_data_size {
                data_size
            } else {
                min_data_size
            };
            Some(Sectors(
                data_space.saturating_sub(*self.filesystem_logical_size_sum()),
            ))
        } else {
            None
        };

        if let Some(Sectors(0)) = remaining_space.as_ref() {
            return Ok(HashMap::default());
        };

        scope(|s| {
            // This collect is needed to ensure all threads are spawned in
            // parallel, not each thread being spawned and immediately joined
            // in the next iterator step which would result in sequential
            // iteration.
            #[allow(clippy::needless_collect)]
            let handles = self.filesystems.iter_mut()
                .filter_map(|(name, uuid, fs)| {
                    if let Some(mt_pt) = fs.should_extend() {
                        let extend_size = StratFilesystem::extend_size(
                            fs.thindev_size(),
                            remaining_space.as_mut(),
                        );
                        if extend_size == Sectors(0) {
                            None
                        } else {
                            Some((
                                name,
                                *uuid,
                                fs,
                                mt_pt,
                                extend_size,
                            ))
                        }
                    } else {
                        None
                    }
                })
                .map(|(name, uuid, fs, mt_pt, extend_size)| {
                    s.spawn(move |_| -> StratisResult<_> {
                        let diff = fs.handle_extension(&mt_pt, extend_size)?;
                        Ok((name, uuid, fs, diff))
                    })
                })
                .collect::<Vec<_>>();

            let needs_save = handles.into_iter()
                .filter_map(|h| {
                    h.join()
                        .map_err(|_| {
                            warn!("Failed to get status of filesystem extension");
                        })
                        .ok()
                })
                .fold(Vec::new(), |mut acc, res| {
                    match res {
                        Ok((name, uuid, fs, diff)) => {
                            updated.insert(uuid, diff);
                            acc.push((name, uuid, fs));
                        }
                        Err(e) => {
                            warn!("Failed to extend filesystem: {}", e);
                        }
                    }
                    acc
                });

            let mdv = &self.mdv;
            // This collect is needed to ensure all threads are spawned in
            // parallel, not each thread being spawned and immediately joined
            // in the next iterator step which would result in sequential
            // iteration.
            #[allow(clippy::needless_collect)]
            let handles = needs_save.into_iter()
                .map(|(name, uuid, fs)| {
                    s.spawn(move |_| {
                        if let Err(e) = mdv.save_fs(name, uuid, fs) {
                            error!("Could not save MDV for fs with UUID {} and name {} belonging to pool with UUID {}, reason: {:?}",
                                        uuid, name, pool_uuid, e);
                        }
                    })
                })
                .collect::<Vec<_>>();
           handles.into_iter().for_each(|h| if h.join().is_err() {
                warn!("Failed to get status of MDV save");
           });
        })
        .map_err(|_| StratisError::Msg("Failed to retrieve parallel filesystem extension status".to_string()))?;

        if remaining_space == Some(Sectors(0)) {
            warn!(
                "Overprovisioning protection must be disabled or more space must be added to the pool to extend the filesystem further"
            );
        }

        Ok(updated)
    }

    /// Set the current status of the thin_pool device to thin_pool_status.
    /// If there has been a change, log that change at the info or warn level
    /// as appropriate.
    fn set_state(&mut self, thin_pool_status: Option<ThinPoolStatus>) {
        let current_status = self.thin_pool_status;
        let new_status: Option<ThinPoolStatusDigest> = thin_pool_status.as_ref().map(|s| s.into());

        if current_status != new_status {
            let current_status_str = current_status
                .map(|x| x.to_string())
                .unwrap_or_else(|| "none".to_string());

            if new_status != Some(ThinPoolStatusDigest::Good) {
                warn!(
                    "Status of thinpool device with \"{}\" changed from \"{}\" to \"{}\"",
                    thin_pool_identifiers(&self.thin_pool),
                    current_status_str,
                    new_status
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "none".to_string()),
                );
            } else {
                info!(
                    "Status of thinpool device with \"{}\" changed from \"{}\" to \"{}\"",
                    thin_pool_identifiers(&self.thin_pool),
                    current_status_str,
                    new_status
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "none".to_string()),
                );
            }
        }

        self.thin_pool_status = new_status;
        self.used = status_to_usage(thin_pool_status);
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

    /// Set the pool IO mode to queue writes when out of space.
    ///
    /// This mode should be enabled when the pool has space to allocate to the pool.
    /// This prevents unnecessary IO errors while the pools is being extended and
    /// the writes can then be processed after the extension.
    fn set_error_mode(&mut self) -> bool {
        if !self.out_of_alloc_space() {
            if let Err(e) = self.thin_pool.error_if_no_space(get_dm()) {
                warn!(
                    "Could not put thin pool into IO error mode on out of space conditions: {}",
                    e
                );
                false
            } else {
                true
            }
        } else {
            false
        }
    }

    /// Set the pool IO mode to error on writes when out of space.
    ///
    /// This mode should be enabled when the pool is out of space to allocate to the
    /// pool.
    pub fn set_queue_mode(&mut self) -> bool {
        if self.out_of_alloc_space() {
            if let Err(e) = self.thin_pool.queue_if_no_space(get_dm()) {
                warn!(
                    "Could not put thin pool into IO queue mode on out of space conditions: {}",
                    e
                );
                false
            } else {
                true
            }
        } else {
            false
        }
    }

    /// Returns true if the pool has run out of available space to allocate.
    pub fn out_of_alloc_space(&self) -> bool {
        self.thin_pool
            .table()
            .table
            .params
            .feature_args
            .contains("error_if_no_space")
    }

    /// Extend thinpool's data dev. See extend_thin_sub_device for more info.
    ///
    /// Because this method must extend both the data device and metadata device,
    /// both extension sizes are returned as Ok((data_extension, metadata_extension)).
    fn extend_thin_data_device(
        &mut self,
        pool_uuid: PoolUuid,
        backstore: &mut Backstore,
    ) -> (bool, StratisResult<(Sectors, Sectors)>) {
        let mut should_save = false;

        let available_size = backstore.available_in_backstore();
        let res = calculate_subdevice_extension(
            available_size,
            self.thin_pool.data_dev().size(),
            self.thin_pool.meta_dev().size(),
            datablocks_to_sectors(DATA_ALLOC_SIZE),
            self.fs_limit,
        );
        if let Err(StratisError::OutOfSpaceError(_)) = res {
            should_save |= self.set_error_mode();
        }
        let (requested_data, requested_meta) = match res {
            Ok((rd, rm)) => (rd, rm),
            Err(e) => return (should_save, Err(e)),
        };

        let res = ThinPool::extend_thin_sub_devices(
            pool_uuid,
            &mut self.thin_pool,
            backstore,
            Some((requested_data, &mut self.segments.data_segments)),
            (
                requested_meta,
                &mut self.segments.meta_segments,
                &mut self.segments.meta_spare_segments,
            ),
        );
        match res {
            Ok((Sectors(0), Sectors(0))) | Err(_) => (should_save, res),
            Ok(_) => (should_save | res.is_ok(), res),
        }
    }

    /// Extend thinpool's meta dev. See extend_thin_sub_device for more info.
    fn extend_thin_meta_device(
        &mut self,
        pool_uuid: PoolUuid,
        backstore: &mut Backstore,
        new_thin_limit: u64,
    ) -> (bool, StratisResult<Sectors>) {
        let new_meta_size = match thin_metadata_size(
            DATA_BLOCK_SIZE,
            self.thin_pool.data_dev().size(),
            new_thin_limit,
        ) {
            Ok(nms) => nms,
            Err(e) => return (false, Err(e)),
        };
        let current_meta_size = self.thin_pool.meta_dev().size();

        if Sectors(new_meta_size.saturating_sub(*current_meta_size))
            > backstore.available_in_backstore()
        {
            (
                self.set_error_mode(),
                Err(StratisError::Msg(
                    "Not enough unallocated space available on the pool to extend metadata device"
                        .to_string(),
                )),
            )
        } else if new_meta_size != current_meta_size {
            let ext = ThinPool::extend_thin_sub_devices(
                pool_uuid,
                &mut self.thin_pool,
                backstore,
                None,
                (
                    Sectors(new_meta_size.saturating_sub(*current_meta_size)),
                    &mut self.segments.meta_segments,
                    &mut self.segments.meta_spare_segments,
                ),
            )
            .map(|(_, meta_ext)| meta_ext);
            (ext.is_ok(), ext)
        } else {
            (false, Ok(Sectors(0)))
        }
    }

    /// Extend the thinpool's data and meta devices. The amount returned may be 0, if
    /// nothing could be allocated. Sets the existing segs passed in through the _info
    /// parameter to the new value that specifies the arrangement of segments on the
    /// extended device.
    fn extend_thin_sub_devices(
        pool_uuid: PoolUuid,
        thinpooldev: &mut ThinPoolDev,
        backstore: &mut Backstore,
        data_info: Option<(Sectors, &mut Vec<(Sectors, Sectors)>)>,
        meta_info: MetaInfo<'_>,
    ) -> StratisResult<(Sectors, Sectors)> {
        let mut empty_segs = Vec::new();
        let (data_extend_size, data_existing_segments) = match data_info {
            Some((des, ds)) => (des, ds),
            None => (Sectors(0), &mut empty_segs),
        };
        let (meta_extend_size, meta_existing_segments, spare_meta_existing_segments) = meta_info;

        if data_extend_size == Sectors(0) && meta_extend_size == Sectors(0) {
            info!("No room left for device extension");
            return Ok((Sectors(0), Sectors(0)));
        }

        thinpooldev.suspend(get_dm(), DmOptions::default())?;

        if data_extend_size != Sectors(0) {
            info!(
                "Attempting to extend thinpool data sub-device belonging to pool {} by {}",
                pool_uuid, data_extend_size
            );
        }
        if meta_extend_size != Sectors(0) {
            info!(
                "Attempting to extend thinpool meta sub-device belonging to pool {} by {}",
                pool_uuid, meta_extend_size
            );
        }

        let device = backstore
            .device()
            .expect("If request succeeded, backstore must have cap device.");

        let mut requests = Vec::new();
        let mut data_index = None;
        let mut meta_index = None;
        if data_extend_size != Sectors(0) {
            requests.push(data_extend_size);
            data_index = Some(0);
        }
        if meta_extend_size != Sectors(0) {
            // Metadata area extension
            requests.push(meta_extend_size);
            // Spare metadata area extension
            requests.push(meta_extend_size);
            meta_index = Some(data_index.map(|i| (i + 1, i + 2)).unwrap_or((0, 1)));
        }

        match backstore.request_alloc(&requests) {
            Ok(Some(transaction)) => {
                let backstore_segs = transaction.get_backstore();
                backstore.commit_alloc(pool_uuid, transaction)?;

                // meta_segments.0 is the existing metadata area
                // meta_segments.1 is the spare metadata area
                let (data_segment, meta_segments) = (
                    data_index.and_then(|i| backstore_segs.get(i).cloned()),
                    meta_index.and_then(|(m, sm)| {
                        backstore_segs
                            .get(m)
                            .and_then(|seg| backstore_segs.get(sm).map(|seg_s| (*seg, *seg_s)))
                    }),
                );
                let (data_segments, meta_and_spare_segments) = (
                    data_segment.map(|seg| coalesce_segs(data_existing_segments, &[seg])),
                    meta_segments.map(|(seg, seg_s)| {
                        (
                            coalesce_segs(meta_existing_segments, &[seg]),
                            coalesce_segs(spare_meta_existing_segments, &[seg_s]),
                        )
                    }),
                );

                // Meta extension must be done first because growing the data device
                // first could cause inadequate metadata space if the metadata
                // extension fails.
                if let Some((mut ms, mut sms)) = meta_and_spare_segments {
                    // Leaves meta device suspended
                    thinpooldev.set_meta_table(get_dm(), segs_to_table(device, &ms))?;

                    meta_existing_segments.clear();
                    meta_existing_segments.append(&mut ms);

                    spare_meta_existing_segments.clear();
                    spare_meta_existing_segments.append(&mut sms);
                }

                if let Some(mut ds) = data_segments {
                    // Leaves data device suspended
                    thinpooldev.set_data_table(get_dm(), segs_to_table(device, &ds))?;

                    data_existing_segments.clear();
                    data_existing_segments.append(&mut ds);
                }

                thinpooldev.resume(get_dm())?;

                if let Some(seg) = data_segment {
                    info!(
                        "Extended thinpool data sub-device belonging to pool with uuid {} by {}",
                        pool_uuid, seg.1
                    );
                }
                if let Some(seg) = meta_segments {
                    info!(
                        "Extended thinpool meta sub-device belonging to pool with uuid {} by {}",
                        pool_uuid, seg.0 .1
                    );
                }

                Ok((
                    data_segment.map(|seg| seg.1).unwrap_or(Sectors(0)),
                    meta_segments.map(|(seg, _)| seg.1).unwrap_or(Sectors(0)),
                ))
            }
            Ok(None) => Ok((Sectors(0), Sectors(0))),
            Err(err) => {
                error!(
                    "Attempted to extend a thinpool sub-device belonging to pool with uuid {} but failed with error: {:?}",
                    pool_uuid,
                    err
                );
                Err(err)
            }
        }
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

    pub fn filesystems(&self) -> Vec<(Name, FilesystemUuid, &StratFilesystem)> {
        self.filesystems
            .iter()
            .map(|(name, uuid, x)| (name.clone(), *uuid, x))
            .collect()
    }

    pub fn filesystems_mut(&mut self) -> Vec<(Name, FilesystemUuid, &mut StratFilesystem)> {
        self.filesystems
            .iter_mut()
            .map(|(name, uuid, x)| (name.clone(), *uuid, x))
            .collect()
    }

    /// Create a filesystem within the thin pool. Given name must not
    /// already be in use.
    pub fn create_filesystem(
        &mut self,
        pool_name: &str,
        pool_uuid: PoolUuid,
        name: &str,
        size: Sectors,
    ) -> StratisResult<FilesystemUuid> {
        let (fs_uuid, mut new_filesystem) =
            StratFilesystem::initialize(pool_uuid, &self.thin_pool, size, self.id_gen.new_id()?)?;
        let name = Name::new(name.to_owned());
        if let Err(err) = self.mdv.save_fs(&name, fs_uuid, &new_filesystem) {
            if let Err(err2) = retry_with_index(Fixed::from_millis(100).take(4), |i| {
                trace!(
                    "Cleanup new filesystem after failed save_fs() attempt {}",
                    i
                );
                new_filesystem.destroy(&self.thin_pool)
            }) {
                error!(
                    "When handling failed save_fs(), fs.destroy() failed: {}",
                    err2
                )
            }
            return Err(err);
        }
        self.filesystems.insert(name, fs_uuid, new_filesystem);
        let (name, fs) = self
            .filesystems
            .get_by_uuid(fs_uuid)
            .expect("Inserted above");
        fs.udev_fs_change(pool_name, fs_uuid, &name);

        Ok(fs_uuid)
    }

    /// Create a filesystem snapshot of the origin.  Given origin_uuid
    /// must exist.  Returns the Uuid of the new filesystem.
    pub fn snapshot_filesystem(
        &mut self,
        pool_name: &str,
        pool_uuid: PoolUuid,
        origin_uuid: FilesystemUuid,
        snapshot_name: &str,
    ) -> StratisResult<(FilesystemUuid, &mut StratFilesystem)> {
        let snapshot_fs_uuid = FilesystemUuid::new_v4();
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
                return Err(StratisError::Msg(
                    "snapshot_filesystem failed, filesystem not found".into(),
                ));
            }
        };
        let new_fs_name = Name::new(snapshot_name.to_owned());
        self.mdv
            .save_fs(&new_fs_name, snapshot_fs_uuid, &new_filesystem)?;
        self.filesystems
            .insert(new_fs_name, snapshot_fs_uuid, new_filesystem);
        let (new_fs_name, fs) = self
            .filesystems
            .get_by_uuid(snapshot_fs_uuid)
            .expect("Inserted above");
        fs.udev_fs_change(pool_name, snapshot_fs_uuid, &new_fs_name);
        Ok((
            snapshot_fs_uuid,
            self.filesystems
                .get_mut_by_uuid(snapshot_fs_uuid)
                .expect("just inserted")
                .1,
        ))
    }

    /// Destroy a filesystem within the thin pool. Destroy metadata associated
    /// with the thinpool. If there is a failure to destroy the filesystem,
    /// retain it, and return an error.
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
    pub fn state(&self) -> Option<&ThinPoolStatusDigest> {
        self.thin_pool_status.as_ref()
    }

    /// Rename a filesystem within the thin pool.
    ///
    /// * Ok(Some(true)) is returned if the filesystem was successfully renamed.
    /// * Ok(Some(false)) is returned if the source and target filesystem names are the same
    /// * Ok(None) is returned if the source filesystem name does not exist
    /// * An error is returned if the target filesystem name already exists
    pub fn rename_filesystem(
        &mut self,
        pool_name: &str,
        uuid: FilesystemUuid,
        new_name: &str,
    ) -> StratisResult<Option<bool>> {
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
            self.filesystems.insert(new_name, uuid, filesystem);
            let (new_name, fs) = self.filesystems.get_by_uuid(uuid).expect("Inserted above");
            fs.udev_fs_change(pool_name, uuid, &new_name);
            Ok(Some(true))
        }
    }

    /// The names of DM devices belonging to this pool that may generate events
    pub fn get_eventing_dev_names(&self, pool_uuid: PoolUuid) -> Vec<DmNameBuf> {
        let mut eventing = vec![
            format_flex_ids(pool_uuid, FlexRole::ThinMeta).0,
            format_flex_ids(pool_uuid, FlexRole::ThinData).0,
            format_flex_ids(pool_uuid, FlexRole::MetadataVolume).0,
            format_thinpool_ids(pool_uuid, ThinPoolRole::Pool).0,
        ];
        eventing.extend(
            self.filesystems
                .iter()
                .map(|(_, uuid, _)| format_thin_ids(pool_uuid, ThinRole::Filesystem(*uuid)).0),
        );
        eventing
    }

    /// Suspend the thinpool
    pub fn suspend(&mut self) -> StratisResult<()> {
        // thindevs automatically suspended when thinpool is suspended
        self.thin_pool.suspend(get_dm(), DmOptions::default())?;
        // If MDV suspend fails, resume the thin pool and return the error
        if let Err(err) = self.mdv.suspend() {
            if let Err(e) = self.thin_pool.resume(get_dm()) {
                Err(StratisError::Chained(
                    "Suspending the MDV failed and MDV suspend clean up action of resuming the thin pool also failed".to_string(),
                    // NOTE: This should potentially put the pool in maintenance-only
                    // mode. For now, this will have no effect.
                    Box::new(StratisError::NoActionRollbackError{
                        causal_error: Box::new(err),
                        rollback_error: Box::new(StratisError::from(e)),
                    }),
                ))
            } else {
                Err(err)
            }
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

    pub fn fs_limit(&self) -> u64 {
        self.fs_limit
    }

    pub fn set_fs_limit(
        &mut self,
        pool_uuid: PoolUuid,
        backstore: &mut Backstore,
        new_limit: u64,
    ) -> (bool, StratisResult<()>) {
        if self.fs_limit >= new_limit {
            return (
                false,
                Err(StratisError::Msg(
                    "New filesystem limit must be greater than the current limit".to_string(),
                )),
            );
        }
        let max_fs_limit = match self.mdv.max_fs_limit() {
            Ok(m) => m,
            Err(e) => return (false, Err(e)),
        };
        if new_limit > max_fs_limit {
            (
                false,
                Err(StratisError::Msg(format!(
                    "Currently Stratis can only handle a filesystem limit of up to {}",
                    max_fs_limit
                ))),
            )
        } else {
            let (should_save, res) = self.extend_thin_meta_device(pool_uuid, backstore, new_limit);
            if res.is_ok() {
                self.fs_limit = new_limit;
            }
            (should_save, res.map(|_| ()))
        }
    }

    /// Returns a boolean indicating whether overprovisioning is disabled or not.
    pub fn overprov_enabled(&self) -> bool {
        self.enable_overprov
    }

    /// Set the overprovisioning mode to either enabled or disabled based on the boolean
    /// provided as an input and return an error if changing this property fails.
    pub fn set_overprov_mode(
        &mut self,
        backstore: &Backstore,
        enabled: bool,
    ) -> (bool, StratisResult<()>) {
        if self.enable_overprov && !enabled {
            let data_size = self.thin_pool.data_dev().size();
            let min_data_size = match self.min_datadev_size(backstore) {
                Ok(min) => min,
                Err(e) => return (false, Err(e)),
            };
            let data_limit = if self.out_of_alloc_space() || data_size > min_data_size {
                data_size
            } else {
                min_data_size
            };

            if self.filesystem_logical_size_sum() > data_limit {
                (false, Err(StratisError::Msg(format!(
                    "Cannot disable overprovisioning on a pool that is already overprovisioned; the sum of the logical sizes of all filesystems and snapshots must be less than the data space available to the thin pool ({}) to disable overprovisioning",
                    data_limit
                ))))
            } else {
                self.enable_overprov = false;
                (true, Ok(()))
            }
        } else if !self.enable_overprov && enabled {
            self.enable_overprov = true;
            (true, Ok(()))
        } else {
            (false, Ok(()))
        }
    }

    /// Determine the minimum data size possible to ensure that a filesystem can be
    /// safely extended when overprovisioning is disabled regardless of future
    /// requirements.
    pub fn min_datadev_size(&self, backstore: &Backstore) -> StratisResult<Sectors> {
        let (data, _) = divide_space(
            backstore.available_in_backstore(),
            self.thin_pool.data_dev().size(),
            self.thin_pool.meta_dev().size(),
            self.mdv.max_fs_limit()?,
        )?;

        Ok(data)
    }
}

impl<'a> Into<Value> for &'a ThinPool {
    fn into(self) -> Value {
        json!({
            "filesystems": Value::Array(
                self.filesystems.iter()
                    .map(|(name, uuid, fs)| {
                        let mut json = Map::new();
                        json.insert("name".to_string(), Value::from(name.to_string()));
                        json.insert("uuid".to_string(), Value::from(uuid.to_string()));
                        if let Value::Object(map) = fs.into() {
                            json.extend(map.into_iter());
                        } else {
                                panic!("SimFilesystem::into() always returns JSON object")
                        }
                        Value::from(json)
                    })
                    .collect()
            )
        })
    }
}

/// Represents the attributes of the thin pool that are being watched for changes.
pub struct ThinPoolState {
    allocated_size: Bytes,
    used: Option<Bytes>,
}

impl StateDiff for ThinPoolState {
    type Diff = ThinPoolDiff;

    fn diff(&self, new_state: &Self) -> Self::Diff {
        ThinPoolDiff {
            allocated_size: self.allocated_size.compare(&new_state.allocated_size),
            used: self.used.compare(&new_state.used),
        }
    }
}

impl<'a> DumpState<'a> for ThinPool {
    type State = ThinPoolState;
    type DumpInput = &'a Backstore;

    fn cached(&self) -> Self::State {
        ThinPoolState {
            allocated_size: self.allocated_size.bytes(),
            used: self.total_physical_used().map(|u| u.bytes()),
        }
    }

    fn dump(&mut self, input: Self::DumpInput) -> Self::State {
        let state = self.thin_pool.status(get_dm(), DmOptions::default()).ok();
        self.set_state(state);
        self.allocated_size = input.datatier_allocated_size();
        ThinPoolState {
            allocated_size: self.allocated_size.bytes(),
            used: self.total_physical_used().map(|u| u.bytes()),
        }
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
            feature_args: Some(self.thin_pool.table().table.params.feature_args.clone()),
            fs_limit: Some(self.fs_limit),
            enable_overprov: Some(self.enable_overprov),
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
        if let Err(e) = thin_check(&meta_dev.devnode()) {
            warn!("Thin check failed: {}", e);
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

    use nix::mount::{mount, MsFlags};

    use devicemapper::{Bytes, SECTOR_SIZE};

    use crate::engine::{
        engine::Filesystem,
        shared::DEFAULT_THIN_DEV_SIZE,
        strat_engine::{
            cmd,
            metadata::MDADataSize,
            tests::{loopbacked, real},
            writing::SyncAll,
        },
    };

    use super::*;

    #[allow(clippy::cast_possible_truncation)]
    const BYTES_PER_WRITE: usize = 2 * IEC::Ki as usize * SECTOR_SIZE as usize;

    /// Test lazy allocation.
    /// Verify that ThinPool::new() succeeds.
    /// Verify that the starting size is equal to the calculated initial size params.
    /// Verify that check on an empty pool does not increase the allocation size.
    /// Create filesystems on the thin pool until the low water mark is passed.
    /// Verify that the data and metadata devices have been extended by the calculated
    /// increase amount.
    /// Verify that the total allocated size is equal to the size of all flex devices
    /// added together.
    /// Verify that the metadata device is the size equal to the output of
    /// thin_metadata_size.
    fn test_lazy_allocation(paths: &[&Path]) {
        let pool_uuid = PoolUuid::new_v4();

        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MDADataSize::default(), None).unwrap();

        let size = ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap();
        let mut pool = ThinPool::new(pool_uuid, &size, DATA_BLOCK_SIZE, &mut backstore).unwrap();

        let init_data_size = size.data_size();
        let init_meta_size = size.meta_size();

        assert_eq!(init_data_size, pool.thin_pool.data_dev().size());
        assert_eq!(init_meta_size, pool.thin_pool.meta_dev().size());

        // This confirms that the check method does not increase the size until
        // the data low water mark is hit.
        pool.check(pool_uuid, &mut backstore).unwrap();

        assert_eq!(init_data_size, pool.thin_pool.data_dev().size());
        assert_eq!(init_meta_size, pool.thin_pool.meta_dev().size());

        let (data_ext, meta_ext) = calculate_subdevice_extension(
            backstore.available_in_backstore(),
            pool.thin_pool.data_dev().size(),
            pool.thin_pool.meta_dev().size(),
            datablocks_to_sectors(DATA_ALLOC_SIZE),
            DEFAULT_FS_LIMIT,
        )
        .unwrap();

        let mut i = 0;
        loop {
            pool.create_filesystem(
                "testpool",
                pool_uuid,
                format!("testfs{}", i).as_str(),
                Sectors(2 * IEC::Gi),
            )
            .unwrap();
            i += 1;

            let init_used = pool.data_used().unwrap();
            let init_data_size = pool.thin_pool.data_dev().size();
            let (changed, diff) = pool.check(pool_uuid, &mut backstore).unwrap();
            if init_data_size - init_used < datablocks_to_sectors(DATA_LOWATER) {
                assert!(changed);
                assert!(diff.allocated_size.is_changed());
                break;
            }
        }

        assert_eq!(init_data_size + data_ext, pool.thin_pool.data_dev().size());
        assert_eq!(init_meta_size + meta_ext, pool.thin_pool.meta_dev().size());
        assert_eq!(
            pool.thin_pool.meta_dev().size(),
            thin_metadata_size(
                DATA_BLOCK_SIZE,
                pool.thin_pool.data_dev().size(),
                DEFAULT_FS_LIMIT,
            )
            .unwrap()
        );
        assert_eq!(
            backstore.datatier_allocated_size(),
            pool.thin_pool.data_dev().size()
                + pool.thin_pool.meta_dev().size() * 2u64
                + pool.mdv.device().size()
        );
    }

    #[test]
    fn loop_test_lazy_allocation() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, Some(Sectors(10 * IEC::Mi))),
            test_lazy_allocation,
        );
    }

    #[test]
    fn real_test_lazy_allocation() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, Some(Sectors(10 * IEC::Mi)), None),
            test_lazy_allocation,
        );
    }

    /// Verify that a full pool extends properly when additional space is added.
    fn test_full_pool(paths: &[&Path]) {
        let pool_name = "pool";
        let pool_uuid = PoolUuid::new_v4();
        let (first_path, remaining_paths) = paths.split_at(1);
        let mut backstore =
            Backstore::initialize(pool_uuid, first_path, MDADataSize::default(), None).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        )
        .unwrap();

        let fs_uuid = pool
            .create_filesystem(
                pool_name,
                pool_uuid,
                "stratis_test_filesystem",
                DEFAULT_THIN_DEV_SIZE,
            )
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
                convert_test!(IEC::Mi, u64, usize),
                OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(file_path)
                    .unwrap(),
            );
            // Write the write_buf until the pool is full
            loop {
                match pool
                    .thin_pool
                    .status(get_dm(), DmOptions::default())
                    .unwrap()
                {
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
        match pool
            .thin_pool
            .status(get_dm(), DmOptions::default())
            .unwrap()
        {
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
        match pool
            .thin_pool
            .status(get_dm(), DmOptions::default())
            .unwrap()
        {
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
            &loopbacked::DeviceLimits::Exactly(2, Some(Bytes::from(IEC::Gi * 2).sectors())),
            test_full_pool,
        );
    }

    #[test]
    fn real_test_full_pool() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(
                2,
                Some(Bytes::from(IEC::Gi * 2).sectors()),
                Some(Bytes::from(IEC::Gi * 4).sectors()),
            ),
            test_full_pool,
        );
    }

    /// Verify a snapshot has the same files and same contents as the origin.
    fn test_filesystem_snapshot(paths: &[&Path]) {
        let pool_name = "pool";
        let pool_uuid = PoolUuid::new_v4();
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MDADataSize::default(), None).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        )
        .unwrap();

        let filesystem_name = "stratis_test_filesystem";
        let fs_uuid = pool
            .create_filesystem(pool_name, pool_uuid, filesystem_name, DEFAULT_THIN_DEV_SIZE)
            .unwrap();

        cmd::udev_settle().unwrap();

        assert!(Path::new(&format!("/dev/stratis/{}/{}", pool_name, filesystem_name)).exists());

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
                    convert_test!(IEC::Mi, u64, usize),
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

        let snapshot_name = "test_snapshot";
        let (_, snapshot_filesystem) = pool
            .snapshot_filesystem(pool_name, pool_uuid, fs_uuid, snapshot_name)
            .unwrap();

        cmd::udev_settle().unwrap();

        // Assert both symlinks are still present.
        assert!(Path::new(&format!("/dev/stratis/{}/{}", pool_name, filesystem_name)).exists());
        assert!(Path::new(&format!("/dev/stratis/{}/{}", pool_name, snapshot_name)).exists());

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

        let pool_uuid = PoolUuid::new_v4();
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MDADataSize::default(), None).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        )
        .unwrap();

        let pool_name = "stratis_test_pool";
        let fs_uuid = pool
            .create_filesystem(pool_name, pool_uuid, name1, DEFAULT_THIN_DEV_SIZE)
            .unwrap();

        cmd::udev_settle().unwrap();

        assert!(Path::new(&format!("/dev/stratis/{}/{}", pool_name, name1)).exists());

        let action = pool.rename_filesystem(pool_name, fs_uuid, name2).unwrap();

        cmd::udev_settle().unwrap();

        // Check that the symlink has been renamed.
        assert!(!Path::new(&format!("/dev/stratis/{}/{}", pool_name, name1)).exists());
        assert!(Path::new(&format!("/dev/stratis/{}/{}", pool_name, name2)).exists());

        assert_matches!(action, Some(_));
        let flexdevs: FlexDevsSave = pool.record();
        let thinpoolsave: ThinPoolDevSave = pool.record();

        retry_operation!(pool.teardown());

        let pool =
            ThinPool::setup(pool_name, pool_uuid, &thinpoolsave, &flexdevs, &backstore).unwrap();

        assert_eq!(&*pool.get_filesystem_by_uuid(fs_uuid).unwrap().0, name2);
    }

    #[test]
    fn loop_test_filesystem_rename() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 4, None),
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
        let pool_name = "pool";
        let pool_uuid = PoolUuid::new_v4();
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MDADataSize::default(), None).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        )
        .unwrap();

        let fs_uuid = pool
            .create_filesystem(pool_name, pool_uuid, "fsname", DEFAULT_THIN_DEV_SIZE)
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

        let new_pool = ThinPool::setup(
            pool_name,
            pool_uuid,
            &thinpooldevsave,
            &pool.record(),
            &backstore,
        )
        .unwrap();

        assert!(new_pool.get_filesystem_by_uuid(fs_uuid).is_some());
    }

    #[test]
    fn loop_test_pool_setup() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 4, None),
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
        let pool_uuid = PoolUuid::new_v4();
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MDADataSize::default(), None).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        )
        .unwrap();
        let pool_name = "stratis_test_pool";
        let fs_name = "stratis_test_filesystem";
        let fs_uuid = pool
            .create_filesystem(pool_name, pool_uuid, fs_name, DEFAULT_THIN_DEV_SIZE)
            .unwrap();

        retry_operation!(pool.destroy_filesystem(pool_name, fs_uuid));
        let flexdevs: FlexDevsSave = pool.record();
        let thinpooldevsave: ThinPoolDevSave = pool.record();
        pool.teardown().unwrap();

        // Check that destroyed fs is not present in MDV. If the record
        // had been left on the MDV that didn't match a thin_id in the
        // thinpool, ::setup() will fail.
        let pool = ThinPool::setup(
            pool_name,
            pool_uuid,
            &thinpooldevsave,
            &flexdevs,
            &backstore,
        )
        .unwrap();

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

    /// Just suspend and resume the device and make sure it doesn't crash.
    /// Suspend twice in succession and then resume twice in succession
    /// to check idempotency.
    fn test_suspend_resume(paths: &[&Path]) {
        let pool_name = "pool";
        let pool_uuid = PoolUuid::new_v4();
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MDADataSize::default(), None).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        )
        .unwrap();

        pool.create_filesystem(
            pool_name,
            pool_uuid,
            "stratis_test_filesystem",
            DEFAULT_THIN_DEV_SIZE,
        )
        .unwrap();

        pool.suspend().unwrap();
        pool.suspend().unwrap();
        pool.resume().unwrap();
        pool.resume().unwrap();
    }

    #[test]
    fn loop_test_suspend_resume() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 4, None),
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

        let pool_name = "pool";
        let pool_uuid = PoolUuid::new_v4();
        let mut backstore =
            Backstore::initialize(pool_uuid, paths2, MDADataSize::default(), None).unwrap();
        let mut pool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        )
        .unwrap();

        let fs_uuid = pool
            .create_filesystem(
                pool_name,
                pool_uuid,
                "stratis_test_filesystem",
                DEFAULT_THIN_DEV_SIZE,
            )
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
            &loopbacked::DeviceLimits::Range(3, 4, None),
            test_set_device,
        );
    }

    #[test]
    fn real_test_set_device() {
        real::test_with_spec(&real::DeviceLimits::AtLeast(2, None, None), test_set_device);
    }

    /// Test the case where the metadata does not grow at all when increased by
    /// `available_space` and the data device size added to available space is a
    /// divisible by `DATA_BLOCK_SIZE`. This previously triggered a bug where the
    /// upper limit would be equal to the total size and trigger an assertion.
    #[test]
    fn test_divide_space() {
        divide_space(
            Sectors(2 * IEC::Ki),
            Sectors(20 * IEC::Ki),
            Sectors(808),
            100,
        )
        .unwrap();
    }
}

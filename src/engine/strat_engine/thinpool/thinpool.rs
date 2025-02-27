// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle management of a pool's thinpool device.

use std::{
    cmp::{max, min, Ordering},
    collections::{hash_map::Entry, HashMap, HashSet},
    marker::PhantomData,
    thread::scope,
};

use itertools::Itertools;
use retry::{delay::Fixed, retry_with_index};
use serde_json::{Map, Value};

use devicemapper::{
    device_exists, message, Bytes, DataBlocks, Device, DmDevice, DmName, DmNameBuf, DmOptions,
    LinearDev, MetaBlocks, Sectors, ThinDevId, ThinPoolDev, ThinPoolStatus, IEC,
};

use crate::{
    engine::{
        engine::{DumpState, Filesystem, StateDiff},
        strat_engine::{
            backstore::backstore::{v1, v2, InternalBackstore},
            cmd::{set_uuid, thin_check, thin_metadata_size, thin_repair},
            dm::{get_dm, list_of_thin_pool_devices, remove_optional_devices},
            names::{
                format_flex_ids, format_thin_ids, format_thinpool_ids, FlexRole, ThinPoolRole,
                ThinRole,
            },
            serde_structs::{FlexDevsSave, Recordable, ThinPoolDevSave},
            shared::merge,
            thinpool::{
                dm_structs::{
                    linear_table, thin_pool_status_parser, thin_table, ThinPoolStatusDigest,
                },
                filesystem::StratFilesystem,
                mdv::MetadataVol,
                thinids::ThinDevIdPool,
            },
            writing::wipe_sectors,
        },
        structures::Table,
        types::{
            Compare, Diff, FilesystemUuid, Name, OffsetDirection, PoolUuid, SetDeleteAction,
            StratFilesystemDiff, ThinPoolDiff,
        },
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

#[derive(strum_macros::AsRefStr)]
#[strum(serialize_all = "snake_case")]
enum FeatureArg {
    ErrorIfNoSpace,
    NoDiscardPassdown,
    SkipBlockZeroing,
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

/// Calculate the room available for data that is not taken up by metadata.
fn room_for_data(usable_size: Sectors, meta_size: Sectors) -> Sectors {
    Sectors(
        usable_size
            .saturating_sub(*INITIAL_MDV_SIZE)
            .saturating_sub(*meta_size * 2u64),
    )
}

pub struct ThinPoolSizeParams {
    meta_size: MetaBlocks,
    data_size: DataBlocks,
    mdv_size: Sectors,
}

impl ThinPoolSizeParams {
    /// Create a new set of initial sizes for all flex devices.
    pub fn new(total_usable: Sectors) -> StratisResult<Self> {
        let meta_size = thin_metadata_size(DATA_BLOCK_SIZE, total_usable, DEFAULT_FS_LIMIT)?;
        let data_size = min(
            room_for_data(total_usable, meta_size),
            datablocks_to_sectors(DATA_ALLOC_SIZE),
        );

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
pub struct ThinPool<B> {
    thin_pool: ThinPoolDev,
    segments: Segments,
    id_gen: ThinDevIdPool,
    filesystems: Table<FilesystemUuid, StratFilesystem>,
    mdv: MetadataVol,
    /// The single DM device that the backstore presents as its upper-most
    /// layer. All DM components obtain their storage from this layer.
    /// The device will change if the backstore adds or removes a cache.
    backstore_device: Device,
    thin_pool_status: Option<ThinPoolStatus>,
    allocated_size: Sectors,
    fs_limit: u64,
    enable_overprov: bool,
    out_of_meta_space: bool,
    backstore: PhantomData<B>,
}

impl<B> ThinPool<B> {
    /// Get the last cached value for the total amount of space used on the pool.
    /// Stratis metadata size will be added a layer about my StratPool.
    pub fn total_physical_used(&self) -> Option<Sectors> {
        calc_total_physical_used(self.used().map(|(du, _)| du), &self.segments)
    }

    /// Get the last cached value for the total amount of space used on the
    /// thin pool in the data and metadata devices.
    fn used(&self) -> Option<(Sectors, MetaBlocks)> {
        self.thin_pool_status
            .as_ref()
            .and_then(thin_pool_status_parser::used)
            .map(|(d, m)| (datablocks_to_sectors(d), m))
    }

    /// Sum the logical size of all filesystems on the pool.
    pub fn filesystem_logical_size_sum(&self) -> StratisResult<Sectors> {
        Ok(self
            .mdv
            .filesystems()?
            .iter()
            .map(|fssave| fssave.size)
            .sum())
    }

    /// Set the current status of the thin_pool device to thin_pool_status.
    /// If there has been a change, log that change at the info or warn level
    /// as appropriate.
    fn set_state(&mut self, thin_pool_status: Option<ThinPoolStatus>) {
        let current_status = self.thin_pool_status.as_ref().map(|s| s.into());
        let new_status: Option<ThinPoolStatusDigest> = thin_pool_status.as_ref().map(|s| s.into());

        if current_status != new_status {
            let current_status_str = current_status
                .as_ref()
                .map(|x| x.as_ref())
                .unwrap_or_else(|| "none");

            if new_status != Some(ThinPoolStatusDigest::Good) {
                warn!(
                    "Status of thinpool device with \"{}\" changed from \"{}\" to \"{}\"",
                    thin_pool_identifiers(&self.thin_pool),
                    current_status_str,
                    new_status
                        .as_ref()
                        .map(|s| s.as_ref())
                        .unwrap_or_else(|| "none"),
                );
            } else {
                info!(
                    "Status of thinpool device with \"{}\" changed from \"{}\" to \"{}\"",
                    thin_pool_identifiers(&self.thin_pool),
                    current_status_str,
                    new_status
                        .as_ref()
                        .map(|s| s.as_ref())
                        .unwrap_or_else(|| "none"),
                );
            }
        }

        self.thin_pool_status = thin_pool_status;
    }

    /// Tear down the components managed here: filesystems, the MDV,
    /// and the actual thinpool device itself.
    ///
    /// Err(_) contains a tuple with a bool as the second element indicating whether or not
    /// there are filesystems that were unable to be torn down. This distinction exists because
    /// if filesystems remain, the pool could receive IO and should remain in set up pool data
    /// structures. However if all filesystems were torn down, the pool can be moved to
    /// the designation of partially constructed pools as no IO can be received on the pool
    /// and it has been partially torn down.
    pub fn teardown(&mut self, pool_uuid: PoolUuid) -> Result<(), (StratisError, bool)> {
        let fs_uuids = self
            .filesystems
            .iter()
            .map(|(_, fs_uuid, _)| *fs_uuid)
            .collect::<Vec<_>>();

        // Must succeed in tearing down all filesystems before the
        // thinpool..
        for fs_uuid in fs_uuids {
            StratFilesystem::teardown(pool_uuid, fs_uuid).map_err(|e| (e, true))?;
            self.filesystems.remove_by_uuid(fs_uuid);
        }
        let devs = list_of_thin_pool_devices(pool_uuid);
        remove_optional_devices(devs).map_err(|e| (e, false))?;

        // ..but MDV has no DM dependencies with the above
        self.mdv.teardown(pool_uuid).map_err(|e| (e, false))?;

        Ok(())
    }

    /// Set the pool IO mode to error on writes when out of space.
    ///
    /// This mode should be enabled when the pool is out of space to allocate to the
    /// pool.
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

    /// Set the pool IO mode to queue writes when out of space.
    ///
    /// This mode should be enabled when the pool has space to allocate to the pool.
    /// This prevents unnecessary IO errors while the pools is being extended and
    /// the writes can then be processed after the extension.
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
        thin_table::get_feature_args(self.thin_pool.table())
            .contains(&FeatureArg::ErrorIfNoSpace.as_ref().to_string())
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
        size_limit: Option<Sectors>,
    ) -> StratisResult<FilesystemUuid> {
        if self
            .mdv
            .filesystems()?
            .into_iter()
            .map(|fssave| fssave.name)
            .collect::<HashSet<_>>()
            .contains(name)
        {
            return Err(StratisError::Msg(format!(
                "Pool {pool_name} already has a record of filesystem name {name}"
            )));
        }

        let (fs_uuid, mut new_filesystem) = StratFilesystem::initialize(
            pool_uuid,
            &self.thin_pool,
            size,
            size_limit,
            self.id_gen.new_id()?,
        )?;
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
        assert!(self.get_filesystem_by_name(snapshot_name).is_none());
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
                origin_uuid,
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
    fn destroy_filesystem(
        &mut self,
        pool_name: &str,
        uuid: FilesystemUuid,
    ) -> StratisResult<Option<FilesystemUuid>> {
        match self.filesystems.remove_by_uuid(uuid) {
            Some((fs_name, mut fs)) => match fs.destroy(&self.thin_pool) {
                Ok(_) => {
                    self.clear_out_of_meta_flag();
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
    pub fn state(&self) -> Option<ThinPoolStatusDigest> {
        self.thin_pool_status.as_ref().map(|s| s.into())
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

    pub fn fs_limit(&self) -> u64 {
        self.fs_limit
    }

    /// Returns a boolean indicating whether overprovisioning is disabled or not.
    pub fn overprov_enabled(&self) -> bool {
        self.enable_overprov
    }

    /// Indicate to the pool that it may now have more room for metadata growth.
    pub fn clear_out_of_meta_flag(&mut self) {
        self.out_of_meta_space = false;
    }

    /// Calculate filesystem metadata from current state
    pub fn current_fs_metadata(&self, fs_name: Option<&str>) -> StratisResult<String> {
        serde_json::to_string(
            &self
                .filesystems
                .iter()
                .filter_map(|(name, uuid, fs)| {
                    if fs_name.map(|n| *n == **name).unwrap_or(true) {
                        Some((*uuid, fs.record(name, *uuid)))
                    } else {
                        None
                    }
                })
                .collect::<HashMap<_, _>>(),
        )
        .map_err(|e| e.into())
    }

    /// Read filesystem metadata from mdv
    pub fn last_fs_metadata(&self, fs_name: Option<&str>) -> StratisResult<String> {
        serde_json::to_string(
            &self
                .mdv
                .filesystems()?
                .iter()
                .filter_map(|fssave| {
                    if fs_name.map(|n| *n == fssave.name).unwrap_or(true) {
                        Some((fssave.uuid, fssave))
                    } else {
                        None
                    }
                })
                .collect::<HashMap<_, _>>(),
        )
        .map_err(|e| e.into())
    }
}

impl ThinPool<v1::Backstore> {
    /// Make a new thin pool.
    #[cfg(any(test, feature = "extras"))]
    pub fn new(
        pool_uuid: PoolUuid,
        thin_pool_size: &ThinPoolSizeParams,
        data_block_size: Sectors,
        backstore: &mut v1::Backstore,
    ) -> StratisResult<ThinPool<v1::Backstore>> {
        let mut segments_list = backstore
            .alloc(
                pool_uuid,
                &[
                    thin_pool_size.meta_size(),
                    thin_pool_size.meta_size(),
                    thin_pool_size.data_size(),
                    thin_pool_size.mdv_size(),
                ],
            )?
            .ok_or_else(|| {
                let err_msg = "Could not allocate sufficient space for thinpool devices";
                StratisError::Msg(err_msg.into())
            })?;

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
            linear_table::segs_to_table(backstore_device, &[meta_segments]),
        )?;

        // Wipe the first 4 KiB, i.e. 8 sectors as recommended in kernel DM
        // docs: device-mapper/thin-provisioning.txt: Setting up a fresh
        // pool device.
        wipe_sectors(
            meta_dev.devnode(),
            Sectors(0),
            min(Sectors(8), meta_dev.size()),
        )?;

        let (dm_name, dm_uuid) = format_flex_ids(pool_uuid, FlexRole::ThinData);
        let data_dev = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            linear_table::segs_to_table(backstore_device, &[data_segments]),
        )?;

        let (dm_name, dm_uuid) = format_flex_ids(pool_uuid, FlexRole::MetadataVolume);
        let mdv_dev = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            linear_table::segs_to_table(backstore_device, &[mdv_segments]),
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
                FeatureArg::NoDiscardPassdown.as_ref().to_string(),
                FeatureArg::SkipBlockZeroing.as_ref().to_string(),
            ],
        )?;

        let thin_pool_status = thinpool_dev.status(get_dm(), DmOptions::default()).ok();
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
            thin_pool_status,
            allocated_size: backstore.datatier_allocated_size(),
            fs_limit: DEFAULT_FS_LIMIT,
            enable_overprov: true,
            out_of_meta_space: false,
            backstore: PhantomData,
        })
    }
}

impl ThinPool<v2::Backstore> {
    /// Make a new thin pool.
    pub fn new(
        pool_uuid: PoolUuid,
        thin_pool_size: &ThinPoolSizeParams,
        data_block_size: Sectors,
        backstore: &mut v2::Backstore,
    ) -> StratisResult<ThinPool<v2::Backstore>> {
        let mut segments_list = backstore
            .alloc(
                pool_uuid,
                &[
                    thin_pool_size.meta_size(),
                    thin_pool_size.meta_size(),
                    thin_pool_size.data_size(),
                    thin_pool_size.mdv_size(),
                ],
            )?
            .ok_or_else(|| {
                let err_msg = "Could not allocate sufficient space for thinpool devices";
                StratisError::Msg(err_msg.into())
            })?;

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
            linear_table::segs_to_table(backstore_device, &[meta_segments]),
        )?;

        // Wipe the first 4 KiB, i.e. 8 sectors as recommended in kernel DM
        // docs: device-mapper/thin-provisioning.txt: Setting up a fresh
        // pool device.
        wipe_sectors(
            meta_dev.devnode(),
            Sectors(0),
            min(Sectors(8), meta_dev.size()),
        )?;

        let (dm_name, dm_uuid) = format_flex_ids(pool_uuid, FlexRole::ThinData);
        let data_dev = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            linear_table::segs_to_table(backstore_device, &[data_segments]),
        )?;

        let (dm_name, dm_uuid) = format_flex_ids(pool_uuid, FlexRole::MetadataVolume);
        let mdv_dev = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            linear_table::segs_to_table(backstore_device, &[mdv_segments]),
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
                FeatureArg::NoDiscardPassdown.as_ref().to_string(),
                FeatureArg::SkipBlockZeroing.as_ref().to_string(),
            ],
        )?;

        let thin_pool_status = thinpool_dev.status(get_dm(), DmOptions::default()).ok();
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
            thin_pool_status,
            allocated_size: backstore.datatier_allocated_size(),
            fs_limit: DEFAULT_FS_LIMIT,
            enable_overprov: true,
            out_of_meta_space: false,
            backstore: PhantomData,
        })
    }
}

impl<B> ThinPool<B>
where
    B: 'static + InternalBackstore,
{
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
        backstore: &B,
    ) -> StratisResult<ThinPool<B>> {
        let mdv_segments = flex_devs.meta_dev.to_vec();
        let meta_segments = flex_devs.thin_meta_dev.to_vec();
        let data_segments = flex_devs.thin_data_dev.to_vec();
        let spare_segments = flex_devs.thin_meta_dev_spare.to_vec();

        let backstore_device = backstore.device().expect("When stratisd was running previously, space was allocated from the backstore, so backstore must have a cap device");

        let (dm_name, dm_uuid) = format_flex_ids(pool_uuid, FlexRole::MetadataVolume);
        let mdv_dev = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            linear_table::segs_to_table(backstore_device, &mdv_segments),
        )?;
        let mdv = MetadataVol::setup(pool_uuid, mdv_dev)?;

        let mut filesystem_metadata_map = HashMap::new();
        let mut names = HashSet::new();
        for fs in mdv.filesystems()?.drain(..) {
            if !names.insert(fs.name.to_string()) {
                return Err(StratisError::Msg(format!(
                    "Two filesystems with the same name, {:}, found in filesystem metadata",
                    fs.name
                )));
            }

            let fs_uuid = *fs.uuid;
            if filesystem_metadata_map.insert(fs_uuid, fs).is_some() {
                return Err(StratisError::Msg(format!(
                    "Two filesystems with the same UUID, {fs_uuid}, found in filesystem metadata"
                )));
            }
        }

        let (duplicates_scheduled, mut ready_to_merge, origins, snaps_to_merge) = filesystem_metadata_map
            .values()
            .filter(|md| md.merge)
            .fold((HashMap::new(), HashMap::new(), HashSet::new(), HashSet::new()), |(mut dups_sched, mut ready_to_merge, mut origins, mut snaps_to_merge), md| {
                match md.origin {
                    None => {
                        warn!("Filesystem with UUID {:} and name {:} which has no origin has been scheduled to be merged; this makes no sense.", md.uuid, md.name);
                    }
                    Some(origin) => {
                        let fs_uuid = md.uuid;
                        match dups_sched.entry(origin) {
                            Entry::Vacant(o_dups) => {
                                match ready_to_merge.entry(origin) {
                                    Entry::Vacant(o_ready) => {
                                        o_ready.insert(fs_uuid);
                                        origins.insert(origin);
                                        snaps_to_merge.insert(fs_uuid);
                                    },
                                    Entry::Occupied(o_ready) => {
                                        o_dups.insert(vec![fs_uuid]);
                                        o_ready.remove_entry();
                                        origins.remove(&origin);
                                    },
                                }
                            },
                            Entry::Occupied(o_dups) => {
                                o_dups.into_mut().push(fs_uuid);
                            }
                        }
                    }
                };
                (dups_sched, ready_to_merge, origins, snaps_to_merge)                });

        if !duplicates_scheduled.is_empty() {
            let msg_string = duplicates_scheduled
                .iter()
                .map(|(origin, ss)| {
                    format!(
                        "{:} -> {:}",
                        ss.iter()
                            .map(|u| u.to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                        origin
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            warn!("Ambiguous revert request; at least two snapshots scheduled to be reverted into a single origin. The scheduled reverts will not occur. Snapshots: {msg_string}");
        }

        let links = origins
            .intersection(&snaps_to_merge)
            .collect::<HashSet<_>>();
        let ready_to_merge = ready_to_merge
            .drain()
            .filter(|(origin, snap)| {
                if links.contains(origin) || links.contains(snap) {
                    warn!("A chain of reverts that includes {origin} and {snap} has been scheduled. The intended order of reverts is ambiguous, the scheduled revert will not occur.");
                    false
                } else {
                    true
                }
            })
            .map(|(origin, snap)| {
                (
                    filesystem_metadata_map.remove(&origin).expect("origin and snap sets are disjoint, have no duplicates"),
                    filesystem_metadata_map.remove(&snap).expect("origin and snap sets are disjoint, have no duplicates")
                )
            })
            .collect::<Vec<_>>();

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
            linear_table::segs_to_table(backstore_device, &data_segments),
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
                .map(|hs| hs.to_vec())
                .unwrap_or_else(|| {
                    migrate = true;
                    vec![
                        FeatureArg::NoDiscardPassdown.as_ref().to_string(),
                        FeatureArg::SkipBlockZeroing.as_ref().to_string(),
                        FeatureArg::ErrorIfNoSpace.as_ref().to_string(),
                    ]
                }),
        )?;

        // TODO: Remove in stratisd 4.0.
        if migrate {
            thinpool_dev.queue_if_no_space(get_dm())?;
        }

        let mut fs_table = Table::default();
        for (origin, snap) in ready_to_merge {
            assert!(!origin.merge);
            let merged = merge(&origin, &snap);

            match StratFilesystem::setup(pool_uuid, &thinpool_dev, &merged) {
                Ok(fs) => {
                    if let Err(e) = set_uuid(&fs.devnode(), merged.uuid) {
                        error!(
                            "Could not set the UUID of the XFS filesystem on the Stratis filesystem with UUID {} after revert, reason: {e:?}",
                            merged.uuid
                        );
                    };
                    fs.udev_fs_change(pool_name, merged.uuid, &merged.name);

                    let name = Name::new(merged.name.to_owned());
                    if let Err(e) = mdv.save_fs(&name, merged.uuid, &fs) {
                        error!(
                            "Could not save MDV for fs with UUID {} and name {} belonging to pool with UUID {pool_uuid} after revert, reason: {e:?}",
                            merged.uuid, merged.name
                        );
                    }
                    if let Err(e) = mdv.rm_fs(snap.uuid) {
                        error!(
                            "Could not remove old MDV for fs with UUID {} belonging to pool with UUID {pool_uuid} after revert, reason: {e:?}",
                            snap.uuid
                        );
                    };
                    assert!(
                        fs_table.insert(name, merged.uuid, fs).is_none(),
                        "Duplicates already removed when building filesystem_metadata_map"
                    );
                    if let Err(e) = message(
                        get_dm(),
                        &thinpool_dev,
                        &format!("delete {:}", origin.thin_id),
                    ) {
                        warn!(
                            "Failed to delete space allocated for deleted origin filesystem with UUID {:} and thin id {:}: {e:?} after revert",
                            origin.uuid, origin.thin_id
                        );
                    }
                    for fs in filesystem_metadata_map.values_mut() {
                        if fs.origin.map(|o| o == snap.uuid).unwrap_or(false) {
                            fs.origin = Some(origin.uuid);
                        }
                    }
                }
                Err(err) => {
                    warn!(
                        "Snapshot {snap:?} could not be reverted into origin {origin:?}, reason: {err:?}"
                    );
                    filesystem_metadata_map.insert(*origin.uuid, origin);
                    filesystem_metadata_map.insert(*snap.uuid, snap);
                }
            }
        }

        for fssave in filesystem_metadata_map.values() {
            match StratFilesystem::setup(pool_uuid, &thinpool_dev, fssave) {
                Ok(fs) => {
                    fs.udev_fs_change(pool_name, fssave.uuid, &fssave.name);
                    assert!(
                        fs_table
                            .insert(Name::new(fssave.name.to_owned()), fssave.uuid, fs)
                            .is_none(),
                        "Duplicates already removed when building filesystem_metadata_map"
                    );
                }
                Err(err) => {
                    warn!(
                        "Filesystem specified by metadata {fssave:?} could not be setup, reason: {err:?}"
                    );
                }
            }
        }

        let thin_ids: Vec<ThinDevId> = fs_table.iter().map(|(_, _, fs)| fs.thin_id()).collect();
        let thin_pool_status = thinpool_dev.status(get_dm(), DmOptions::default()).ok();
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
            thin_pool_status,
            allocated_size: backstore.datatier_allocated_size(),
            fs_limit,
            enable_overprov: thin_pool_save.enable_overprov.unwrap_or(true),
            out_of_meta_space: false,
            backstore: PhantomData,
        })
    }

    /// Run status checks and take actions on the thinpool and its components.
    /// The boolean in the return value indicates if a configuration change requiring a
    /// metadata save has been made.
    pub fn check(
        &mut self,
        pool_uuid: PoolUuid,
        backstore: &mut B,
    ) -> StratisResult<(bool, ThinPoolDiff)> {
        assert_eq!(
            backstore.device().expect(
                "thinpool exists and has been allocated to, so backstore must have a cap device"
            ),
            self.backstore_device
        );

        let mut should_save: bool = false;

        let old_state = self.cached();

        // This block will only perform an extension if the check() method
        // is being called when block devices have been newly added to the pool or
        // the metadata low water mark has been reached.
        if !self.out_of_meta_space {
            match self.extend_thin_meta_device(
                pool_uuid,
                backstore,
                None,
                self.used()
                    .and_then(|(_, mu)| {
                        self.thin_pool_status
                            .as_ref()
                            .and_then(thin_pool_status_parser::meta_lowater)
                            .map(|ml| self.thin_pool.meta_dev().size().metablocks() - mu < ml)
                    })
                    .unwrap_or(false),
            ) {
                (changed, Ok(_)) => {
                    should_save |= changed;
                }
                (changed, Err(e)) => {
                    should_save |= changed;
                    warn!("Device extension failed: {}", e);
                }
            };
        }

        if let Some((data_usage, _)) = self.used() {
            if self.thin_pool.data_dev().size() - data_usage < datablocks_to_sectors(DATA_LOWATER)
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
                        Sectors(0)
                    }
                };
                should_save |= amount_allocated != Sectors(0);

                self.thin_pool.set_low_water_mark(get_dm(), DATA_LOWATER)?;
                self.resume()?;
            }
        }

        let new_state = self.dump(backstore);

        Ok((should_save, old_state.diff(&new_state)))
    }

    /// Check all filesystems on this thin pool and return which had their sizes
    /// extended, if any. This method should not need to handle thin pool status
    /// because it never alters the thin pool itself.
    pub fn check_fs(
        &mut self,
        pool_uuid: PoolUuid,
        backstore: &B,
    ) -> StratisResult<HashMap<FilesystemUuid, StratFilesystemDiff>> {
        let mut updated = HashMap::default();
        let mut remaining_space = if !self.enable_overprov {
            let sum = self.filesystem_logical_size_sum()?;
            Some(Sectors(
                room_for_data(
                    backstore.datatier_usable_size(),
                    self.thin_pool.meta_dev().size(),
                )
                .saturating_sub(*sum),
            ))
        } else {
            None
        };

        scope(|s| {
            // This collect is needed to ensure all threads are spawned in
            // parallel, not each thread being spawned and immediately joined
            // in the next iterator step which would result in sequential
            // iteration.
            #[allow(clippy::needless_collect)]
            let handles = self
                .filesystems
                .iter_mut()
                .filter_map(|(name, uuid, fs)| {
                    fs.visit_values(remaining_space.as_mut())
                        .map(|(mt_pt, extend_size)| (name, *uuid, fs, mt_pt, extend_size))
                })
                .map(|(name, uuid, fs, mt_pt, extend_size)| {
                    s.spawn(move || -> StratisResult<_> {
                        let diff = fs.handle_fs_changes(&mt_pt, extend_size)?;
                        Ok((name, uuid, fs, diff))
                    })
                })
                .collect::<Vec<_>>();

            let needs_save = handles
                .into_iter()
                .filter_map(|h| {
                    h.join()
                        .map_err(|_| {
                            warn!("Failed to get status of filesystem operation");
                        })
                        .ok()
                })
                .fold(Vec::new(), |mut acc, res| {
                    match res {
                        Ok((name, uuid, fs, diff)) => {
                            if diff.size.is_changed() {
                                acc.push((name, uuid, fs));
                            }
                            if diff.size.is_changed() || diff.used.is_changed() {
                                updated.insert(uuid, diff);
                            }
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
                    s.spawn(move || {
                        if let Err(e) = mdv.save_fs(name, uuid, fs) {
                            error!("Could not save MDV for fs with UUID {} and name {} belonging to pool with UUID {}, reason: {:?}",
                                        uuid, name, pool_uuid, e);
                        }
                    })
                })
                .collect::<Vec<_>>();
            handles.into_iter().for_each(|h| {
                if h.join().is_err() {
                    warn!("Failed to get status of MDV save");
                }
            });
        });

        if remaining_space == Some(Sectors(0)) {
            warn!(
                "Overprovisioning protection must be disabled or more space must be added to the pool to extend the filesystem further"
            );
        }

        Ok(updated)
    }

    /// Extend thinpool's data dev.
    ///
    /// This method returns the extension size as Ok(data_extension).
    fn extend_thin_data_device(
        &mut self,
        pool_uuid: PoolUuid,
        backstore: &mut B,
    ) -> (bool, StratisResult<Sectors>) {
        fn do_extend<B>(
            thinpooldev: &mut ThinPoolDev,
            backstore: &mut B,
            pool_uuid: PoolUuid,
            data_existing_segments: &mut Vec<(Sectors, Sectors)>,
            data_extend_size: Sectors,
        ) -> StratisResult<Sectors>
        where
            B: InternalBackstore,
        {
            info!(
                "Attempting to extend thinpool data sub-device belonging to pool {} by {}",
                pool_uuid, data_extend_size
            );

            let device = backstore
                .device()
                .expect("If request succeeded, backstore must have cap device.");

            let requests = vec![data_extend_size];
            let data_index = 0;
            match backstore.alloc(pool_uuid, &requests) {
                Ok(Some(backstore_segs)) => {
                    let data_segment = backstore_segs.get(data_index).cloned();
                    let data_segments =
                        data_segment.map(|seg| coalesce_segs(data_existing_segments, &[seg]));
                    if let Some(mut ds) = data_segments {
                        thinpooldev.suspend(get_dm(), DmOptions::default())?;
                        // Leaves data device suspended
                        let res = thinpooldev
                            .set_data_table(get_dm(), linear_table::segs_to_table(device, &ds));

                        if res.is_ok() {
                            data_existing_segments.clear();
                            data_existing_segments.append(&mut ds);
                        }

                        thinpooldev.resume(get_dm())?;

                        res?;
                    }

                    if let Some(seg) = data_segment {
                        info!(
                            "Extended thinpool data sub-device belonging to pool with uuid {} by {}",
                            pool_uuid, seg.1
                        );
                    }

                    Ok(data_segment.map(|seg| seg.1).unwrap_or(Sectors(0)))
                }
                Ok(None) => Ok(Sectors(0)),
                Err(err) => {
                    error!(
                        "Attempted to extend a thinpool data sub-device belonging to pool with uuid {pool_uuid} but failed with error: {err:?}"
                    );
                    Err(err)
                }
            }
        }

        let available_size = backstore.available_in_backstore();
        let data_ext = min(sectors_to_datablocks(available_size), DATA_ALLOC_SIZE);
        if data_ext == DataBlocks(0) {
            return (
                self.set_error_mode(),
                Err(StratisError::OutOfSpaceError(format!(
                    "{DATA_ALLOC_SIZE} requested but no space is available"
                ))),
            );
        }

        let res = do_extend(
            &mut self.thin_pool,
            backstore,
            pool_uuid,
            &mut self.segments.data_segments,
            datablocks_to_sectors(data_ext),
        );

        match res {
            Ok(Sectors(0)) | Err(_) => (false, res),
            Ok(_) => (true, res),
        }
    }

    /// Extend thinpool's meta dev.
    ///
    /// If is_lowater is true, it was determined that the low water mark has been
    /// crossed for metadata and the device size should be doubled instead of
    /// recalculated via thin_metadata_size.
    fn extend_thin_meta_device(
        &mut self,
        pool_uuid: PoolUuid,
        backstore: &mut B,
        new_thin_limit: Option<u64>,
        is_lowater: bool,
    ) -> (bool, StratisResult<Sectors>) {
        fn do_extend<B>(
            thinpooldev: &mut ThinPoolDev,
            backstore: &mut B,
            pool_uuid: PoolUuid,
            meta_existing_segments: &mut Vec<(Sectors, Sectors)>,
            spare_meta_existing_segments: &mut Vec<(Sectors, Sectors)>,
            meta_extend_size: Sectors,
        ) -> StratisResult<Sectors>
        where
            B: InternalBackstore,
        {
            info!(
                "Attempting to extend thinpool meta sub-device belonging to pool {} by {}",
                pool_uuid, meta_extend_size
            );

            let device = backstore
                .device()
                .expect("If request succeeded, backstore must have cap device.");

            let requests = vec![meta_extend_size, meta_extend_size];
            let meta_index = 0;
            let spare_index = 1;
            match backstore.alloc(pool_uuid, &requests) {
                Ok(Some(backstore_segs)) => {
                    let meta_and_spare_segment = backstore_segs.get(meta_index).and_then(|seg| {
                        backstore_segs.get(spare_index).map(|seg_s| (*seg, *seg_s))
                    });
                    let meta_and_spare_segments = meta_and_spare_segment.map(|(seg, seg_s)| {
                        (
                            coalesce_segs(meta_existing_segments, &[seg]),
                            coalesce_segs(spare_meta_existing_segments, &[seg_s]),
                        )
                    });

                    if let Some((mut ms, mut sms)) = meta_and_spare_segments {
                        thinpooldev.suspend(get_dm(), DmOptions::default())?;

                        // Leaves meta device suspended
                        let res = thinpooldev
                            .set_meta_table(get_dm(), linear_table::segs_to_table(device, &ms));

                        if res.is_ok() {
                            meta_existing_segments.clear();
                            meta_existing_segments.append(&mut ms);

                            spare_meta_existing_segments.clear();
                            spare_meta_existing_segments.append(&mut sms);
                        }

                        thinpooldev.resume(get_dm())?;

                        res?;
                    }

                    if let Some((seg, _)) = meta_and_spare_segment {
                        info!(
                            "Extended thinpool meta sub-device belonging to pool with uuid {} by {}",
                            pool_uuid, seg.1
                        );
                    }

                    Ok(meta_and_spare_segment
                        .map(|(seg, _)| seg.1)
                        .unwrap_or(Sectors(0)))
                }
                Ok(None) => Ok(Sectors(0)),
                Err(err) => {
                    error!(
                        "Attempted to extend a thinpool meta sub-device belonging to pool with uuid {} but failed with error: {:?}",
                        pool_uuid,
                        err
                    );
                    Err(err)
                }
            }
        }

        let new_meta_size = if is_lowater {
            min(
                2u64 * self.thin_pool.meta_dev().size(),
                backstore.datatier_usable_size(),
            )
        } else {
            match thin_metadata_size(
                DATA_BLOCK_SIZE,
                backstore.datatier_usable_size(),
                new_thin_limit.unwrap_or(self.fs_limit),
            ) {
                Ok(nms) => nms,
                Err(e) => return (false, Err(e)),
            }
        };
        let current_meta_size = self.thin_pool.meta_dev().size();
        let meta_growth = Sectors(new_meta_size.saturating_sub(*current_meta_size));

        if !self.overprov_enabled() && meta_growth > Sectors(0) {
            let sum = match self.filesystem_logical_size_sum() {
                Ok(s) => s,
                Err(e) => {
                    return (false, Err(e));
                }
            };
            let total: Sectors = sum + INITIAL_MDV_SIZE + 2u64 * current_meta_size;
            match total.cmp(&backstore.datatier_usable_size()) {
                Ordering::Less => (),
                Ordering::Equal => {
                    self.out_of_meta_space = true;
                    return (false, Err(StratisError::Msg(
                        "Metadata cannot be extended any further without adding more space or enabling overprovisioning; the sum of filesystem sizes is as large as all space not used for metadata".to_string()
                    )));
                }
                Ordering::Greater => {
                    self.out_of_meta_space = true;
                    return (false, Err(StratisError::Msg(
                        "Detected a size of MDV, filesystem sizes and metadata size that is greater than available space in the pool while overprovisioning is disabled; please file a bug report".to_string()
                    )));
                }
            }
        }

        if 2u64 * meta_growth > backstore.available_in_backstore() {
            self.out_of_meta_space = true;
            (
                self.set_error_mode(),
                Err(StratisError::Msg(
                    "Not enough unallocated space available on the pool to extend metadata device"
                        .to_string(),
                )),
            )
        } else if meta_growth > Sectors(0) {
            let ext = do_extend(
                &mut self.thin_pool,
                backstore,
                pool_uuid,
                &mut self.segments.meta_segments,
                &mut self.segments.meta_spare_segments,
                meta_growth,
            );

            (ext.is_ok(), ext)
        } else {
            (false, Ok(Sectors(0)))
        }
    }

    pub fn set_fs_limit(
        &mut self,
        pool_uuid: PoolUuid,
        backstore: &mut B,
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
                    "Currently Stratis can only handle a filesystem limit of up to {max_fs_limit}"
                ))),
            )
        } else {
            let (mut should_save, res) =
                self.extend_thin_meta_device(pool_uuid, backstore, Some(new_limit), false);
            if res.is_ok() {
                self.fs_limit = new_limit;
                should_save = true;
            }
            (should_save, res.map(|_| ()))
        }
    }

    /// Return the limit for total size of all filesystems when overprovisioning
    /// is disabled.
    pub fn total_fs_limit(&self, backstore: &B) -> Sectors {
        room_for_data(
            backstore.datatier_usable_size(),
            self.thin_pool.meta_dev().size(),
        )
    }

    /// Set the overprovisioning mode to either enabled or disabled based on the boolean
    /// provided as an input and return an error if changing this property fails.
    pub fn set_overprov_mode(&mut self, backstore: &B, enabled: bool) -> (bool, StratisResult<()>) {
        if self.enable_overprov && !enabled {
            let data_limit = self.total_fs_limit(backstore);

            let sum = match self.filesystem_logical_size_sum() {
                Ok(s) => s,
                Err(e) => {
                    return (false, Err(e));
                }
            };
            if sum > data_limit {
                (false, Err(StratisError::Msg(format!(
                    "Cannot disable overprovisioning on a pool that is already overprovisioned; the sum of the logical sizes of all filesystems and snapshots ({sum}) must be less than the data space available to the thin pool ({data_limit}) to disable overprovisioning"
                ))))
            } else {
                self.enable_overprov = false;
                (true, Ok(()))
            }
        } else if !self.enable_overprov && enabled {
            self.enable_overprov = true;
            self.clear_out_of_meta_flag();
            (true, Ok(()))
        } else {
            (false, Ok(()))
        }
    }

    /// Set the filesystem size limit for filesystem with given UUID.
    pub fn set_fs_size_limit(
        &mut self,
        fs_uuid: FilesystemUuid,
        limit: Option<Sectors>,
    ) -> StratisResult<bool> {
        let changed = {
            let (_, fs) = self.get_mut_filesystem_by_uuid(fs_uuid).ok_or_else(|| {
                StratisError::Msg(format!("No filesystem with UUID {fs_uuid} found"))
            })?;
            fs.set_size_limit(limit)?
        };
        let (name, fs) = self
            .get_filesystem_by_uuid(fs_uuid)
            .ok_or_else(|| StratisError::Msg(format!("No filesystem with UUID {fs_uuid} found")))?;
        if changed {
            self.mdv.save_fs(&name, fs_uuid, fs)?;
        }
        Ok(changed)
    }

    /// Set the filesystem merge scheduled value for filesystem with given UUID
    /// Returns true if the value was changed from the filesystem's, previous
    /// value, otherwise false.
    pub fn set_fs_merge_scheduled(
        &mut self,
        fs_uuid: FilesystemUuid,
        scheduled: bool,
    ) -> StratisResult<bool> {
        let (_, fs) = self
            .get_filesystem_by_uuid(fs_uuid)
            .ok_or_else(|| StratisError::Msg(format!("No filesystem with UUID {fs_uuid} found")))?;

        let origin = fs.origin().ok_or_else(|| {
            StratisError::Msg(format!(
                "Filesystem {fs_uuid} has no origin, revert cannot be scheduled or unscheduled"
            ))
        })?;

        if fs.merge_scheduled() == scheduled {
            return Ok(false);
        }

        if scheduled {
            if self
                .get_filesystem_by_uuid(origin)
                .map(|(_, fs)| fs.merge_scheduled())
                .unwrap_or(false)
            {
                return Err(StratisError::Msg(format!(
                    "Filesystem {fs_uuid} is scheduled to replace filesystem {origin}, but filesystem {origin} is already scheduled to replace another filesystem. Since the order in which the filesystems should replace each other is unknown, this operation can not be performed."
                )));
            }

            let (others_scheduled, into_scheduled) =
                self.filesystems
                    .iter()
                    .fold((Vec::new(), Vec::new()), |mut acc, (u, n, f)| {
                        if f.origin().map(|o| o == origin).unwrap_or(false) && f.merge_scheduled() {
                            acc.0.push((u, n, f));
                        }
                        if f.origin().map(|o| o == fs_uuid).unwrap_or(false) && f.merge_scheduled()
                        {
                            acc.1.push((u, n, f));
                        }
                        acc
                    });

            if let Some((n, u, _)) = others_scheduled.first() {
                return Err(StratisError::Msg(format!(
                    "Filesystem {n} with UUID {u} is already scheduled to be reverted into origin filesystem {origin}, unwilling to schedule two revert operations on the same origin filesystem"
                )));
            }

            if let Some((n, u, _)) = into_scheduled.first() {
                return Err(StratisError::Msg(format!(
                    "Filesystem {n} with UUID {u} is already scheduled to be reverted into this filesystem {origin}. The ordering is ambiguous, unwilling to schedule a revert"
                )));
            }
        }

        assert!(
            self.get_mut_filesystem_by_uuid(fs_uuid)
                .expect("Looked up above")
                .1
                .set_merge_scheduled(scheduled)
                .expect("fs.origin() is not None"),
            "Already returned from this method if value to set is the same as current"
        );

        let (name, fs) = self
            .get_filesystem_by_uuid(fs_uuid)
            .expect("Looked up above");

        self.mdv.save_fs(&name, fs_uuid, fs)?;
        Ok(true)
    }

    pub fn destroy_filesystems(
        &mut self,
        pool_name: &str,
        fs_uuids: &HashSet<FilesystemUuid>,
    ) -> StratisResult<SetDeleteAction<FilesystemUuid, (FilesystemUuid, Option<FilesystemUuid>)>>
    {
        let to_be_merged = fs_uuids
            .iter()
            .filter(|u| {
                self.get_filesystem_by_uuid(**u)
                    .map(|(_, fs)| fs.merge_scheduled())
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();

        if !to_be_merged.is_empty() {
            let err_str = format!("The filesystem destroy operation can not be begun until the revert operations for the following filesystem snapshots have been cancelled: {}", to_be_merged.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(", "));
            return Err(StratisError::Msg(err_str));
        }

        let mut snapshots = self
            .filesystems()
            .iter()
            .filter_map(|(_, u, fs)| {
                fs.origin().and_then(|x| {
                    if fs_uuids.contains(&x) {
                        Some((x, (*u, fs.merge_scheduled())))
                    } else {
                        None
                    }
                })
            })
            .fold(HashMap::new(), |mut acc, (u, v)| {
                acc.entry(u)
                    .and_modify(|e: &mut Vec<(FilesystemUuid, _)>| e.push(v))
                    .or_insert(vec![v]);
                acc
            });

        let scheduled_for_merge = snapshots
            .iter()
            .filter(|(_, snaps)| snaps.iter().any(|(_, scheduled)| *scheduled))
            .collect::<Vec<_>>();
        if !scheduled_for_merge.is_empty() {
            let err_str = format!("The filesystem destroy operation can not be begun until the revert operations for the following filesystem snapshots have been cancelled: {}", scheduled_for_merge.iter().map(|(u, _)| u.to_string()).collect::<Vec<_>>().join(", "));
            return Err(StratisError::Msg(err_str));
        }

        let (mut removed, mut updated_origins) = (Vec::new(), Vec::new());
        for &uuid in fs_uuids {
            if let Some((_, fs)) = self.get_filesystem_by_uuid(uuid) {
                let fs_origin = fs.origin();
                let uuid = self
                    .destroy_filesystem(pool_name, uuid)?
                    .expect("just looked up");
                removed.push(uuid);

                for (sn_uuid, _) in snapshots.remove(&uuid).unwrap_or_else(Vec::new) {
                    // The filesystems may have been removed; any one of
                    // them may also be a filesystem that was scheduled for
                    // removal.
                    if let Some((_, sn)) = self.get_mut_filesystem_by_uuid(sn_uuid) {
                        assert!(
                            sn.set_origin(fs_origin),
                            "A snapshot can only have one origin, so it can be in snapshots.values() only once, so its origin value can be set only once"
                        );
                        updated_origins.push((sn_uuid, fs_origin));

                        let (name, sn) = self.get_filesystem_by_uuid(sn_uuid).expect("just got");
                        self.mdv.save_fs(&name, sn_uuid, sn)?;
                    };
                }
            }
        }

        Ok(SetDeleteAction::new(removed, updated_origins))
    }

    /// Set the device on all DM devices
    pub fn set_device(
        &mut self,
        backstore_device: Device,
        offset: Sectors,
        offset_direction: OffsetDirection,
    ) -> StratisResult<bool> {
        if backstore_device == self.backstore_device {
            return Ok(false);
        }

        let meta_table = linear_table::set_target_device(
            self.thin_pool.meta_dev().table(),
            backstore_device,
            offset,
            offset_direction,
        );
        let data_table = linear_table::set_target_device(
            self.thin_pool.data_dev().table(),
            backstore_device,
            offset,
            offset_direction,
        );
        let mdv_table = linear_table::set_target_device(
            self.mdv.device().table(),
            backstore_device,
            offset,
            offset_direction,
        );

        for (start, _) in self.segments.mdv_segments.iter_mut() {
            match offset_direction {
                OffsetDirection::Forwards => *start += offset,
                OffsetDirection::Backwards => *start -= offset,
            }
        }
        for (start, _) in self.segments.data_segments.iter_mut() {
            match offset_direction {
                OffsetDirection::Forwards => *start += offset,
                OffsetDirection::Backwards => *start -= offset,
            }
        }
        for (start, _) in self.segments.meta_segments.iter_mut() {
            match offset_direction {
                OffsetDirection::Forwards => *start += offset,
                OffsetDirection::Backwards => *start -= offset,
            }
        }
        for (start, _) in self.segments.meta_spare_segments.iter_mut() {
            match offset_direction {
                OffsetDirection::Forwards => *start += offset,
                OffsetDirection::Backwards => *start -= offset,
            }
        }

        self.thin_pool.set_meta_table(get_dm(), meta_table)?;
        self.thin_pool.set_data_table(get_dm(), data_table)?;
        self.mdv.set_table(mdv_table)?;

        self.backstore_device = backstore_device;

        Ok(true)
    }
}

impl<B> Into<Value> for &ThinPool<B> {
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

    fn unchanged(&self) -> Self::Diff {
        ThinPoolDiff {
            allocated_size: Diff::Unchanged(self.allocated_size),
            used: Diff::Unchanged(self.used),
        }
    }
}

impl<'a, B> DumpState<'a> for ThinPool<B>
where
    B: 'static + InternalBackstore,
{
    type State = ThinPoolState;
    type DumpInput = &'a B;

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

impl<B> Recordable<FlexDevsSave> for ThinPool<B> {
    fn record(&self) -> FlexDevsSave {
        self.segments.record()
    }
}

impl<B> Recordable<ThinPoolDevSave> for ThinPool<B> {
    fn record(&self) -> ThinPoolDevSave {
        ThinPoolDevSave {
            data_block_size: self.thin_pool.data_block_size(),
            feature_args: Some(
                thin_table::get_feature_args(self.thin_pool.table())
                    .iter()
                    .sorted()
                    .cloned()
                    .collect(),
            ),
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
        linear_table::segs_to_table(device, &meta_segments),
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
        linear_table::segs_to_table(device, spare_segments),
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

    use devicemapper::{Bytes, ThinPoolStatusSummary, SECTOR_SIZE};

    use crate::engine::{
        engine::Filesystem,
        shared::DEFAULT_THIN_DEV_SIZE,
        strat_engine::{
            backstore::{backstore, ProcessedPathInfos, UnownedDevices},
            cmd,
            metadata::MDADataSize,
            tests::{loopbacked, real},
            writing::SyncAll,
        },
        types::ValidatedIntegritySpec,
    };

    use super::*;

    #[allow(clippy::cast_possible_truncation)]
    const BYTES_PER_WRITE: usize = 2 * IEC::Ki as usize * SECTOR_SIZE;

    fn get_devices(paths: &[&Path]) -> StratisResult<UnownedDevices> {
        ProcessedPathInfos::try_from(paths)
            .map(|ps| ps.unpack())
            .map(|(sds, uds)| {
                sds.error_on_not_empty().unwrap();
                uds
            })
    }

    mod v1 {
        use super::*;

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
            let pool_name = Name::new("pool_name".to_string());

            let devices = get_devices(paths).unwrap();

            let mut backstore = backstore::v1::Backstore::initialize(
                pool_name,
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
            )
            .unwrap();
            let size = ThinPoolSizeParams::new(backstore.datatier_usable_size()).unwrap();
            let mut pool = ThinPool::<backstore::v1::Backstore>::new(
                pool_uuid,
                &size,
                DATA_BLOCK_SIZE,
                &mut backstore,
            )
            .unwrap();

            let init_data_size = size.data_size();
            let init_meta_size = size.meta_size();
            let available_on_start = backstore.available_in_backstore();

            assert_eq!(init_data_size, pool.thin_pool.data_dev().size());
            assert_eq!(init_meta_size, pool.thin_pool.meta_dev().size());

            // This confirms that the check method does not increase the size until
            // the data low water mark is hit.
            pool.check(pool_uuid, &mut backstore).unwrap();

            assert_eq!(init_data_size, pool.thin_pool.data_dev().size());
            assert_eq!(init_meta_size, pool.thin_pool.meta_dev().size());

            let mut i = 0;
            loop {
                pool.create_filesystem(
                    "testpool",
                    pool_uuid,
                    format!("testfs{i}").as_str(),
                    Sectors(2 * IEC::Gi),
                    None,
                )
                .unwrap();
                i += 1;

                let init_used = pool.used().unwrap().0;
                let init_size = pool.thin_pool.data_dev().size();
                let (changed, diff) = pool.check(pool_uuid, &mut backstore).unwrap();
                if init_size - init_used < datablocks_to_sectors(DATA_LOWATER) {
                    assert!(changed);
                    assert!(diff.allocated_size.is_changed());
                    break;
                }
            }

            assert_eq!(
                init_data_size
                    + datablocks_to_sectors(min(
                        DATA_ALLOC_SIZE,
                        sectors_to_datablocks(available_on_start),
                    )),
                pool.thin_pool.data_dev().size(),
            );
            assert_eq!(
                pool.thin_pool.meta_dev().size(),
                thin_metadata_size(
                    DATA_BLOCK_SIZE,
                    backstore.datatier_usable_size(),
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

            let first_devices = get_devices(first_path).unwrap();
            let remaining_devices = get_devices(remaining_paths).unwrap();

            let mut backstore = backstore::v1::Backstore::initialize(
                Name::new(pool_name.to_string()),
                pool_uuid,
                first_devices,
                MDADataSize::default(),
                None,
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v1::Backstore>::new(
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
                    None,
                )
                .unwrap();

            let write_buf = &vec![8u8; BYTES_PER_WRITE].into_boxed_slice();
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
                        .truncate(true)
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
            backstore
                .add_datadevs(
                    Name::new(pool_name.to_string()),
                    pool_uuid,
                    remaining_devices,
                    None,
                )
                .unwrap();
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

            let devices = get_devices(paths).unwrap();

            let mut backstore = backstore::v1::Backstore::initialize(
                Name::new(pool_name.to_string()),
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v1::Backstore>::new(
                pool_uuid,
                &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
                DATA_BLOCK_SIZE,
                &mut backstore,
            )
            .unwrap();

            let filesystem_name = "stratis_test_filesystem";
            let fs_uuid = pool
                .create_filesystem(
                    pool_name,
                    pool_uuid,
                    filesystem_name,
                    DEFAULT_THIN_DEV_SIZE,
                    None,
                )
                .unwrap();

            cmd::udev_settle().unwrap();

            assert!(Path::new(&format!("/dev/stratis/{pool_name}/{filesystem_name}")).exists());

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
                    let file_path = source_tmp_dir.path().join(format!("stratis_test{i}.txt"));
                    let mut f = BufWriter::with_capacity(
                        convert_test!(IEC::Mi, u64, usize),
                        OpenOptions::new()
                            .create(true)
                            .truncate(true)
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
            assert!(Path::new(&format!("/dev/stratis/{pool_name}/{filesystem_name}")).exists());
            assert!(Path::new(&format!("/dev/stratis/{pool_name}/{snapshot_name}")).exists());

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
                    let file_path = snapshot_tmp_dir.path().join(format!("stratis_test{i}.txt"));
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
            let pool_name = Name::new("pool_name".to_string());
            let name1 = "name1";
            let name2 = "name2";

            let pool_uuid = PoolUuid::new_v4();

            let devices = get_devices(paths).unwrap();

            let mut backstore = backstore::v1::Backstore::initialize(
                pool_name,
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v1::Backstore>::new(
                pool_uuid,
                &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
                DATA_BLOCK_SIZE,
                &mut backstore,
            )
            .unwrap();

            let pool_name = "stratis_test_pool";
            let fs_uuid = pool
                .create_filesystem(pool_name, pool_uuid, name1, DEFAULT_THIN_DEV_SIZE, None)
                .unwrap();

            cmd::udev_settle().unwrap();

            assert!(Path::new(&format!("/dev/stratis/{pool_name}/{name1}")).exists());

            let action = pool.rename_filesystem(pool_name, fs_uuid, name2).unwrap();

            cmd::udev_settle().unwrap();

            // Check that the symlink has been renamed.
            assert!(!Path::new(&format!("/dev/stratis/{pool_name}/{name1}")).exists());
            assert!(Path::new(&format!("/dev/stratis/{pool_name}/{name2}")).exists());

            assert_eq!(action, Some(true));
            let flexdevs: FlexDevsSave = pool.record();
            let thinpoolsave: ThinPoolDevSave = pool.record();

            retry_operation!(pool.teardown(pool_uuid));

            let pool = ThinPool::setup(pool_name, pool_uuid, &thinpoolsave, &flexdevs, &backstore)
                .unwrap();

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

            let devices = get_devices(paths).unwrap();

            let mut backstore = backstore::v1::Backstore::initialize(
                Name::new(pool_name.to_string()),
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v1::Backstore>::new(
                pool_uuid,
                &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
                DATA_BLOCK_SIZE,
                &mut backstore,
            )
            .unwrap();

            let fs_uuid = pool
                .create_filesystem(pool_name, pool_uuid, "fsname", DEFAULT_THIN_DEV_SIZE, None)
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
                        .truncate(true)
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
            let pool_name = Name::new("pool_name".to_string());

            let devices = get_devices(paths).unwrap();

            let mut backstore = backstore::v1::Backstore::initialize(
                pool_name,
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v1::Backstore>::new(
                pool_uuid,
                &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
                DATA_BLOCK_SIZE,
                &mut backstore,
            )
            .unwrap();
            let pool_name = "stratis_test_pool";
            let fs_name = "stratis_test_filesystem";
            let fs_uuid = pool
                .create_filesystem(pool_name, pool_uuid, fs_name, DEFAULT_THIN_DEV_SIZE, None)
                .unwrap();

            retry_operation!(pool.destroy_filesystem(pool_name, fs_uuid));
            let flexdevs: FlexDevsSave = pool.record();
            let thinpooldevsave: ThinPoolDevSave = pool.record();
            pool.teardown(pool_uuid).unwrap();

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

            let devices = get_devices(paths).unwrap();

            let mut backstore = backstore::v1::Backstore::initialize(
                Name::new(pool_name.to_string()),
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v1::Backstore>::new(
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
                None,
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

            let devices1 = get_devices(paths1).unwrap();
            let devices = get_devices(paths2).unwrap();

            let mut backstore = backstore::v1::Backstore::initialize(
                Name::new(pool_name.to_string()),
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v1::Backstore>::new(
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
                    None,
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
                    .truncate(true)
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
            backstore
                .init_cache(Name::new(pool_name.to_string()), pool_uuid, devices1, None)
                .unwrap();
            let new_device = backstore
                .device()
                .expect("Space already allocated from backstore, backstore must have device");
            assert_ne!(old_device, new_device);
            pool.set_device(new_device, Sectors(0), OffsetDirection::Forwards)
                .unwrap();
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

        /// Set up thinpool and backstore. Set up filesystem and set size limit.
        /// Write past the halfway mark of the filesystem and check that the filesystem
        /// size limit is respected. Increase the filesystem size limit and check that
        /// it is respected. Remove the filesystem size limit and verify that the
        /// filesystem size doubles. Verify that the filesystem size limit cannot be set
        /// below the current filesystem size.
        fn test_fs_size_limit(paths: &[&Path]) {
            let pool_name = "pool";
            let pool_uuid = PoolUuid::new_v4();

            let devices = get_devices(paths).unwrap();

            let mut backstore = backstore::v1::Backstore::initialize(
                Name::new(pool_name.to_string()),
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v1::Backstore>::new(
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
                    Sectors::from(2400 * IEC::Ki),
                    // 1400 * IEC::Mi
                    Some(Sectors(2800 * IEC::Ki)),
                )
                .unwrap();
            let devnode = {
                let (_, fs) = pool.get_mut_filesystem_by_uuid(fs_uuid).unwrap();
                assert_eq!(fs.size_limit(), Some(Sectors(2800 * IEC::Ki)));
                fs.devnode()
            };

            let tmp_dir = tempfile::Builder::new()
                .prefix("stratis_testing")
                .tempdir()
                .unwrap();
            let new_file = tmp_dir.path().join("stratis_test.txt");
            mount(
                Some(&devnode),
                tmp_dir.path(),
                Some("xfs"),
                MsFlags::empty(),
                None as Option<&str>,
            )
            .unwrap();
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(new_file)
                .unwrap();
            let mut bytes_written = Bytes(0);
            // Write 800 * IEC::Mi
            while bytes_written < Bytes::from(800 * IEC::Mi) {
                file.write_all(&[1; 4096]).unwrap();
                bytes_written += Bytes(4096);
            }
            file.sync_all().unwrap();
            pool.check_fs(pool_uuid, &backstore).unwrap();

            {
                let (_, fs) = pool.get_mut_filesystem_by_uuid(fs_uuid).unwrap();
                assert_eq!(fs.size_limit(), Some(fs.size().sectors()));
            }

            // 1600 * IEC::Mi
            pool.set_fs_size_limit(fs_uuid, Some(Sectors(3200 * IEC::Ki)))
                .unwrap();
            {
                let (_, fs) = pool.get_mut_filesystem_by_uuid(fs_uuid).unwrap();
                assert_eq!(fs.size_limit(), Some(Sectors(3200 * IEC::Ki)));
            }
            let mut bytes_written = Bytes(0);
            // Write 200 * IEC::Mi
            while bytes_written < Bytes::from(200 * IEC::Mi) {
                file.write_all(&[1; 4096]).unwrap();
                bytes_written += Bytes(4096);
            }
            file.sync_all().unwrap();
            pool.check_fs(pool_uuid, &backstore).unwrap();

            {
                let (_, fs) = pool.get_mut_filesystem_by_uuid(fs_uuid).unwrap();
                assert_eq!(fs.size_limit(), Some(fs.size().sectors()));
            }

            {
                let (_, fs) = pool
                    .snapshot_filesystem(pool_name, pool_uuid, fs_uuid, "snapshot")
                    .unwrap();
                assert_eq!(fs.size_limit(), Some(Sectors(3200 * IEC::Ki)));
            }

            pool.set_fs_size_limit(fs_uuid, None).unwrap();
            {
                let (_, fs) = pool.get_mut_filesystem_by_uuid(fs_uuid).unwrap();
                assert_eq!(fs.size_limit(), None);
            }
            let mut bytes_written = Bytes(0);
            // Write 400 * IEC::Mi
            while bytes_written < Bytes::from(400 * IEC::Mi) {
                file.write_all(&[1; 4096]).unwrap();
                bytes_written += Bytes(4096);
            }
            file.sync_all().unwrap();
            pool.check_fs(pool_uuid, &backstore).unwrap();

            {
                let (_, fs) = pool.get_mut_filesystem_by_uuid(fs_uuid).unwrap();
                assert_eq!(fs.size().sectors(), Sectors(6400 * IEC::Ki));
            }

            assert!(pool.set_fs_size_limit(fs_uuid, Some(Sectors(50))).is_err());
        }

        #[test]
        fn loop_test_fs_size_limit() {
            loopbacked::test_with_spec(
                &loopbacked::DeviceLimits::Range(1, 3, Some(Sectors(10 * IEC::Mi))),
                test_fs_size_limit,
            );
        }

        #[test]
        fn real_test_fs_size_limit() {
            real::test_with_spec(
                &real::DeviceLimits::Range(1, 3, Some(Sectors(10 * IEC::Mi)), None),
                test_fs_size_limit,
            );
        }

        /// Verify that destroy_filesystems handles origin and merge
        /// scheduled properties correctly when destroying filesystems.
        fn test_thindev_with_origins(paths: &[&Path]) {
            let default_thin_dev_size = Sectors(2 * IEC::Mi);
            let pool_uuid = PoolUuid::new_v4();
            let pool_name = Name::new("pool".to_string());

            let devices = get_devices(paths).unwrap();

            let mut backstore = backstore::v1::Backstore::initialize(
                pool_name,
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v1::Backstore>::new(
                pool_uuid,
                &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
                DATA_BLOCK_SIZE,
                &mut backstore,
            )
            .unwrap();
            let pool_name = "stratis_test_pool";
            let fs_name = "stratis_test_filesystem";
            let fs_uuid = pool
                .create_filesystem(pool_name, pool_uuid, fs_name, default_thin_dev_size, None)
                .unwrap();

            let (sn1_uuid, sn1) = pool
                .snapshot_filesystem(pool_name, pool_uuid, fs_uuid, "snap1")
                .unwrap();
            assert_matches!(sn1.origin(), Some(uuid) => fs_uuid == uuid);

            let (sn2_uuid, sn2) = pool
                .snapshot_filesystem(pool_name, pool_uuid, fs_uuid, "snap2")
                .unwrap();
            assert_matches!(sn2.origin(), Some(uuid) => fs_uuid == uuid);

            assert!(pool.set_fs_merge_scheduled(fs_uuid, true).is_err());
            assert!(pool.set_fs_merge_scheduled(fs_uuid, false).is_err());
            pool.set_fs_merge_scheduled(sn2_uuid, true).unwrap();
            assert!(pool.set_fs_merge_scheduled(sn1_uuid, true).is_err());
            pool.set_fs_merge_scheduled(sn1_uuid, false).unwrap();

            assert!(pool
                .destroy_filesystems(pool_name, &[sn2_uuid, sn1_uuid].into())
                .is_err());
            assert!(pool
                .destroy_filesystems(pool_name, &[fs_uuid, sn1_uuid].into())
                .is_err());
            pool.set_fs_merge_scheduled(sn2_uuid, false).unwrap();

            retry_operation!(pool.destroy_filesystems(pool_name, &[fs_uuid, sn2_uuid].into()));

            assert_eq!(
                pool.get_filesystem_by_uuid(sn1_uuid)
                    .expect("not destroyed")
                    .1
                    .origin(),
                None
            );
        }

        #[test]
        fn loop_test_thindev_with_origins() {
            loopbacked::test_with_spec(
                &loopbacked::DeviceLimits::Range(2, 3, None),
                test_thindev_with_origins,
            );
        }

        #[test]
        fn real_test_thindev_with_origins() {
            real::test_with_spec(
                &real::DeviceLimits::AtLeast(1, None, None),
                test_thindev_with_origins,
            );
        }
    }

    mod v2 {
        use super::*;

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

            let devices = get_devices(paths).unwrap();

            let mut backstore = backstore::v2::Backstore::initialize(
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
                ValidatedIntegritySpec::default(),
            )
            .unwrap();
            let size = ThinPoolSizeParams::new(backstore.datatier_usable_size()).unwrap();
            let mut pool = ThinPool::<backstore::v2::Backstore>::new(
                pool_uuid,
                &size,
                DATA_BLOCK_SIZE,
                &mut backstore,
            )
            .unwrap();

            let init_data_size = size.data_size();
            let init_meta_size = size.meta_size();
            let available_on_start = backstore.available_in_backstore();

            assert_eq!(init_data_size, pool.thin_pool.data_dev().size());
            assert_eq!(init_meta_size, pool.thin_pool.meta_dev().size());

            // This confirms that the check method does not increase the size until
            // the data low water mark is hit.
            pool.check(pool_uuid, &mut backstore).unwrap();

            assert_eq!(init_data_size, pool.thin_pool.data_dev().size());
            assert_eq!(init_meta_size, pool.thin_pool.meta_dev().size());

            let mut i = 0;
            loop {
                pool.create_filesystem(
                    "testpool",
                    pool_uuid,
                    format!("testfs{i}").as_str(),
                    Sectors(2 * IEC::Gi),
                    None,
                )
                .unwrap();
                i += 1;

                let init_used = pool.used().unwrap().0;
                let init_size = pool.thin_pool.data_dev().size();
                let (changed, diff) = pool.check(pool_uuid, &mut backstore).unwrap();
                if init_size - init_used < datablocks_to_sectors(DATA_LOWATER) {
                    assert!(changed);
                    assert!(diff.allocated_size.is_changed());
                    break;
                }
            }

            assert_eq!(
                init_data_size
                    + datablocks_to_sectors(min(
                        DATA_ALLOC_SIZE,
                        sectors_to_datablocks(available_on_start),
                    )),
                pool.thin_pool.data_dev().size(),
            );
            assert_eq!(
                pool.thin_pool.meta_dev().size(),
                thin_metadata_size(
                    DATA_BLOCK_SIZE,
                    backstore.datatier_usable_size(),
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

            let first_devices = get_devices(first_path).unwrap();
            let remaining_devices = get_devices(remaining_paths).unwrap();

            let mut backstore = backstore::v2::Backstore::initialize(
                pool_uuid,
                first_devices,
                MDADataSize::default(),
                None,
                ValidatedIntegritySpec::default(),
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v2::Backstore>::new(
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
                    None,
                )
                .unwrap();

            let write_buf = &vec![8u8; BYTES_PER_WRITE].into_boxed_slice();
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
                        .truncate(true)
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
            backstore
                .add_datadevs(pool_uuid, remaining_devices)
                .unwrap();
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

            let devices = get_devices(paths).unwrap();

            let mut backstore = backstore::v2::Backstore::initialize(
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
                ValidatedIntegritySpec::default(),
            )
            .unwrap();
            warn!("Available: {}", backstore.available_in_backstore());
            let mut pool = ThinPool::<backstore::v2::Backstore>::new(
                pool_uuid,
                &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
                DATA_BLOCK_SIZE,
                &mut backstore,
            )
            .unwrap();

            let filesystem_name = "stratis_test_filesystem";
            let fs_uuid = pool
                .create_filesystem(
                    pool_name,
                    pool_uuid,
                    filesystem_name,
                    DEFAULT_THIN_DEV_SIZE,
                    None,
                )
                .unwrap();

            cmd::udev_settle().unwrap();

            assert!(Path::new(&format!("/dev/stratis/{pool_name}/{filesystem_name}")).exists());

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
                    let file_path = source_tmp_dir.path().join(format!("stratis_test{i}.txt"));
                    let mut f = BufWriter::with_capacity(
                        convert_test!(IEC::Mi, u64, usize),
                        OpenOptions::new()
                            .create(true)
                            .truncate(true)
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
            assert!(Path::new(&format!("/dev/stratis/{pool_name}/{filesystem_name}")).exists());
            assert!(Path::new(&format!("/dev/stratis/{pool_name}/{snapshot_name}")).exists());

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
                    let file_path = snapshot_tmp_dir.path().join(format!("stratis_test{i}.txt"));
                    let mut f = OpenOptions::new().read(true).open(file_path).unwrap();
                    f.read_exact(&mut read_buf).unwrap();
                    assert_eq!(read_buf[0..SECTOR_SIZE], write_buf[0..SECTOR_SIZE]);
                }
            }
        }

        #[test]
        fn loop_test_filesystem_snapshot() {
            loopbacked::test_with_spec(
                &loopbacked::DeviceLimits::Range(2, 3, Some(Bytes::from(5 * IEC::Gi).sectors())),
                test_filesystem_snapshot,
            );
        }

        #[test]
        fn real_test_filesystem_snapshot() {
            real::test_with_spec(
                &real::DeviceLimits::AtLeast(2, Some(Bytes::from(5 * IEC::Gi).sectors()), None),
                test_filesystem_snapshot,
            );
        }

        /// Verify that a filesystem rename causes the filesystem metadata to be
        /// updated.
        fn test_filesystem_rename(paths: &[&Path]) {
            let name1 = "name1";
            let name2 = "name2";

            let pool_uuid = PoolUuid::new_v4();

            let devices = get_devices(paths).unwrap();

            let mut backstore = backstore::v2::Backstore::initialize(
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
                ValidatedIntegritySpec::default(),
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v2::Backstore>::new(
                pool_uuid,
                &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
                DATA_BLOCK_SIZE,
                &mut backstore,
            )
            .unwrap();

            let pool_name = "stratis_test_pool";
            let fs_uuid = pool
                .create_filesystem(pool_name, pool_uuid, name1, DEFAULT_THIN_DEV_SIZE, None)
                .unwrap();

            cmd::udev_settle().unwrap();

            assert!(Path::new(&format!("/dev/stratis/{pool_name}/{name1}")).exists());

            let action = pool.rename_filesystem(pool_name, fs_uuid, name2).unwrap();

            cmd::udev_settle().unwrap();

            // Check that the symlink has been renamed.
            assert!(!Path::new(&format!("/dev/stratis/{pool_name}/{name1}")).exists());
            assert!(Path::new(&format!("/dev/stratis/{pool_name}/{name2}")).exists());

            assert_eq!(action, Some(true));
            let flexdevs: FlexDevsSave = pool.record();
            let thinpoolsave: ThinPoolDevSave = pool.record();

            retry_operation!(pool.teardown(pool_uuid));

            let pool = ThinPool::setup(pool_name, pool_uuid, &thinpoolsave, &flexdevs, &backstore)
                .unwrap();

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

            let devices = get_devices(paths).unwrap();

            let mut backstore = backstore::v2::Backstore::initialize(
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
                ValidatedIntegritySpec::default(),
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v2::Backstore>::new(
                pool_uuid,
                &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
                DATA_BLOCK_SIZE,
                &mut backstore,
            )
            .unwrap();

            let fs_uuid = pool
                .create_filesystem(pool_name, pool_uuid, "fsname", DEFAULT_THIN_DEV_SIZE, None)
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
                        .truncate(true)
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

            let devices = get_devices(paths).unwrap();

            let mut backstore = backstore::v2::Backstore::initialize(
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
                ValidatedIntegritySpec::default(),
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v2::Backstore>::new(
                pool_uuid,
                &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
                DATA_BLOCK_SIZE,
                &mut backstore,
            )
            .unwrap();
            let pool_name = "stratis_test_pool";
            let fs_name = "stratis_test_filesystem";
            let fs_uuid = pool
                .create_filesystem(pool_name, pool_uuid, fs_name, DEFAULT_THIN_DEV_SIZE, None)
                .unwrap();

            retry_operation!(pool.destroy_filesystem(pool_name, fs_uuid));
            let flexdevs: FlexDevsSave = pool.record();
            let thinpooldevsave: ThinPoolDevSave = pool.record();
            pool.teardown(pool_uuid).unwrap();

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

            let devices = get_devices(paths).unwrap();

            let mut backstore = backstore::v2::Backstore::initialize(
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
                ValidatedIntegritySpec::default(),
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v2::Backstore>::new(
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
                None,
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
        fn test_cache(paths: &[&Path]) {
            assert!(paths.len() > 1);

            let (paths1, paths2) = paths.split_at(paths.len() / 2);

            let pool_name = "pool";
            let pool_uuid = PoolUuid::new_v4();

            let devices1 = get_devices(paths1).unwrap();
            let devices = get_devices(paths2).unwrap();

            let mut backstore = backstore::v2::Backstore::initialize(
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
                ValidatedIntegritySpec::default(),
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v2::Backstore>::new(
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
                    None,
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
                    .truncate(true)
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

            backstore.init_cache(pool_uuid, devices1).unwrap();

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
        fn loop_test_cache() {
            loopbacked::test_with_spec(&loopbacked::DeviceLimits::Range(3, 4, None), test_cache);
        }

        #[test]
        fn real_test_cache() {
            real::test_with_spec(&real::DeviceLimits::AtLeast(2, None, None), test_cache);
        }

        /// Set up thinpool and backstore. Set up filesystem and set size limit.
        /// Write past the halfway mark of the filesystem and check that the filesystem
        /// size limit is respected. Increase the filesystem size limit and check that
        /// it is respected. Remove the filesystem size limit and verify that the
        /// filesystem size doubles. Verify that the filesystem size limit cannot be set
        /// below the current filesystem size.
        fn test_fs_size_limit(paths: &[&Path]) {
            let pool_name = "pool";
            let pool_uuid = PoolUuid::new_v4();

            let devices = get_devices(paths).unwrap();

            let mut backstore = backstore::v2::Backstore::initialize(
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
                ValidatedIntegritySpec::default(),
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v2::Backstore>::new(
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
                    Sectors::from(2400 * IEC::Ki),
                    // 1400 * IEC::Mi
                    Some(Sectors(2800 * IEC::Ki)),
                )
                .unwrap();
            let devnode = {
                let (_, fs) = pool.get_mut_filesystem_by_uuid(fs_uuid).unwrap();
                assert_eq!(fs.size_limit(), Some(Sectors(2800 * IEC::Ki)));
                fs.devnode()
            };

            let tmp_dir = tempfile::Builder::new()
                .prefix("stratis_testing")
                .tempdir()
                .unwrap();
            let new_file = tmp_dir.path().join("stratis_test.txt");
            mount(
                Some(&devnode),
                tmp_dir.path(),
                Some("xfs"),
                MsFlags::empty(),
                None as Option<&str>,
            )
            .unwrap();
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(new_file)
                .unwrap();
            let mut bytes_written = Bytes(0);
            // Write 800 * IEC::Mi
            while bytes_written < Bytes::from(800 * IEC::Mi) {
                file.write_all(&[1; 4096]).unwrap();
                bytes_written += Bytes(4096);
            }
            file.sync_all().unwrap();
            pool.check_fs(pool_uuid, &backstore).unwrap();

            {
                let (_, fs) = pool.get_mut_filesystem_by_uuid(fs_uuid).unwrap();
                assert_eq!(fs.size_limit(), Some(fs.size().sectors()));
            }

            // 1600 * IEC::Mi
            pool.set_fs_size_limit(fs_uuid, Some(Sectors(3200 * IEC::Ki)))
                .unwrap();
            {
                let (_, fs) = pool.get_mut_filesystem_by_uuid(fs_uuid).unwrap();
                assert_eq!(fs.size_limit(), Some(Sectors(3200 * IEC::Ki)));
            }
            let mut bytes_written = Bytes(0);
            // Write 200 * IEC::Mi
            while bytes_written < Bytes::from(200 * IEC::Mi) {
                file.write_all(&[1; 4096]).unwrap();
                bytes_written += Bytes(4096);
            }
            file.sync_all().unwrap();
            pool.check_fs(pool_uuid, &backstore).unwrap();

            {
                let (_, fs) = pool.get_mut_filesystem_by_uuid(fs_uuid).unwrap();
                assert_eq!(fs.size_limit(), Some(fs.size().sectors()));
            }

            {
                let (_, fs) = pool
                    .snapshot_filesystem(pool_name, pool_uuid, fs_uuid, "snapshot")
                    .unwrap();
                assert_eq!(fs.size_limit(), Some(Sectors(3200 * IEC::Ki)));
            }

            pool.set_fs_size_limit(fs_uuid, None).unwrap();
            {
                let (_, fs) = pool.get_mut_filesystem_by_uuid(fs_uuid).unwrap();
                assert_eq!(fs.size_limit(), None);
            }
            let mut bytes_written = Bytes(0);
            // Write 400 * IEC::Mi
            while bytes_written < Bytes::from(400 * IEC::Mi) {
                file.write_all(&[1; 4096]).unwrap();
                bytes_written += Bytes(4096);
            }
            file.sync_all().unwrap();
            pool.check_fs(pool_uuid, &backstore).unwrap();

            {
                let (_, fs) = pool.get_mut_filesystem_by_uuid(fs_uuid).unwrap();
                assert_eq!(fs.size().sectors(), Sectors(6400 * IEC::Ki));
            }

            assert!(pool.set_fs_size_limit(fs_uuid, Some(Sectors(50))).is_err());
        }

        #[test]
        fn loop_test_fs_size_limit() {
            loopbacked::test_with_spec(
                &loopbacked::DeviceLimits::Range(1, 3, Some(Sectors(10 * IEC::Mi))),
                test_fs_size_limit,
            );
        }

        #[test]
        fn real_test_fs_size_limit() {
            real::test_with_spec(
                &real::DeviceLimits::Range(1, 3, Some(Sectors(10 * IEC::Mi)), None),
                test_fs_size_limit,
            );
        }

        /// Verify that destroy_filesystems handles origin and merge
        /// scheduled properties correctly when destroying filesystems.
        fn test_thindev_with_origins(paths: &[&Path]) {
            let default_thin_dev_size = Sectors(2 * IEC::Mi);
            let pool_uuid = PoolUuid::new_v4();

            let devices = get_devices(paths).unwrap();

            let mut backstore = backstore::v2::Backstore::initialize(
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
                ValidatedIntegritySpec::default(),
            )
            .unwrap();
            let mut pool = ThinPool::<backstore::v2::Backstore>::new(
                pool_uuid,
                &ThinPoolSizeParams::new(backstore.available_in_backstore()).unwrap(),
                DATA_BLOCK_SIZE,
                &mut backstore,
            )
            .unwrap();
            let pool_name = "stratis_test_pool";
            let fs_name = "stratis_test_filesystem";
            let fs_uuid = pool
                .create_filesystem(pool_name, pool_uuid, fs_name, default_thin_dev_size, None)
                .unwrap();

            let (sn1_uuid, sn1) = pool
                .snapshot_filesystem(pool_name, pool_uuid, fs_uuid, "snap1")
                .unwrap();
            assert_matches!(sn1.origin(), Some(uuid) => fs_uuid == uuid);

            let (sn2_uuid, sn2) = pool
                .snapshot_filesystem(pool_name, pool_uuid, fs_uuid, "snap2")
                .unwrap();
            assert_matches!(sn2.origin(), Some(uuid) => fs_uuid == uuid);

            assert!(pool.set_fs_merge_scheduled(fs_uuid, true).is_err());
            assert!(pool.set_fs_merge_scheduled(fs_uuid, false).is_err());
            pool.set_fs_merge_scheduled(sn2_uuid, true).unwrap();
            assert!(pool.set_fs_merge_scheduled(sn1_uuid, true).is_err());
            pool.set_fs_merge_scheduled(sn1_uuid, false).unwrap();

            assert!(pool
                .destroy_filesystems(pool_name, &[sn2_uuid, sn1_uuid].into())
                .is_err());
            assert!(pool
                .destroy_filesystems(pool_name, &[fs_uuid, sn1_uuid].into())
                .is_err());
            pool.set_fs_merge_scheduled(sn2_uuid, false).unwrap();

            retry_operation!(pool.destroy_filesystems(pool_name, &[fs_uuid, sn2_uuid].into()));

            assert_eq!(
                pool.get_filesystem_by_uuid(sn1_uuid)
                    .expect("not destroyed")
                    .1
                    .origin(),
                None
            );
        }

        #[test]
        fn loop_test_thindev_with_origins() {
            loopbacked::test_with_spec(
                &loopbacked::DeviceLimits::Range(2, 3, None),
                test_thindev_with_origins,
            );
        }

        #[test]
        fn real_test_thindev_with_origins() {
            real::test_with_spec(
                &real::DeviceLimits::AtLeast(1, None, None),
                test_thindev_with_origins,
            );
        }
    }
}

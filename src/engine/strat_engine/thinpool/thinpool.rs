// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle management of a pool's thinpool device.

#[cfg(feature = "full_runtime")]
use std::borrow::BorrowMut;

#[cfg(feature = "full_runtime")]
use std::cmp;

#[cfg(feature = "full_runtime")]
use uuid::Uuid;

use devicemapper as dm;
use devicemapper::{device_exists, DataBlocks, Device, DmDevice, DmName, FlakeyTargetParams,
                   LinearDev, LinearDevTargetParams, LinearTargetParams, Sectors, TargetLine,
                   ThinDev, ThinDevId, ThinPoolDev, IEC};
#[cfg(feature = "full_runtime")]
use devicemapper::{DmNameBuf, MetaBlocks, ThinPoolStatusSummary};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::engine::Filesystem;
use super::super::super::structures::Table;
use super::super::super::types::{FilesystemUuid, Name, PoolUuid, RenameAction};

use super::super::backstore::Backstore;
use super::super::cmd::{thin_check, thin_repair};
#[cfg(feature = "full_runtime")]
use super::super::device::wipe_sectors;
use super::super::devlinks;
use super::super::dm::get_dm;
use super::super::dmnames::{format_flex_ids, format_thin_ids, format_thinpool_ids, FlexRole,
                            ThinPoolRole, ThinRole};
use super::super::serde_structs::{FlexDevsSave, Recordable, ThinPoolDevSave};

#[cfg(feature = "full_runtime")]
use super::filesystem::FilesystemStatus;
use super::filesystem::StratFilesystem;

use super::mdv::MetadataVol;
use super::thinids::ThinDevIdPool;

pub const DATA_BLOCK_SIZE: Sectors = Sectors(2 * IEC::Ki);
pub const DATA_LOWATER: DataBlocks = DataBlocks(512);

#[cfg(feature = "full_runtime")]
const META_LOWATER: MetaBlocks = MetaBlocks(512);

#[cfg(feature = "full_runtime")]
const DEFAULT_THIN_DEV_SIZE: Sectors = Sectors(2 * IEC::Gi); // 1 TiB

#[cfg(feature = "full_runtime")]
const INITIAL_META_SIZE: MetaBlocks = MetaBlocks(4 * IEC::Ki);

#[cfg(feature = "full_runtime")]
pub const INITIAL_DATA_SIZE: DataBlocks = DataBlocks(768);

#[cfg(feature = "full_runtime")]
const INITIAL_MDV_SIZE: Sectors = Sectors(32 * IEC::Ki); // 16 MiB

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
#[cfg(feature = "full_runtime")]
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

#[cfg(feature = "full_runtime")]
pub struct ThinPoolSizeParams {
    meta_size: MetaBlocks,
    data_size: DataBlocks,
    mdv_size: Sectors,
}

#[cfg(feature = "full_runtime")]
impl ThinPoolSizeParams {
    /// The number of Sectors in the MetaBlocks.
    pub fn meta_size(&self) -> Sectors {
        self.meta_size.sectors()
    }
    /// The number of Sectors in the DataBlocks.
    pub fn data_size(&self) -> Sectors {
        *self.data_size * DATA_BLOCK_SIZE
    }
    /// MDV size
    pub fn mdv_size(&self) -> Sectors {
        self.mdv_size
    }
}

#[cfg(feature = "full_runtime")]
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
    pool_uuid: PoolUuid,
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
}

impl ThinPool {
    /// Make a new thin pool.
    #[cfg(feature = "full_runtime")]
    pub fn new(
        pool_uuid: PoolUuid,
        thin_pool_size: &ThinPoolSizeParams,
        data_block_size: Sectors,
        low_water_mark: DataBlocks,
        backstore: &mut Backstore,
    ) -> StratisResult<ThinPool> {
        let mut segments_list = match backstore.alloc_space(&[
            thin_pool_size.meta_size(),
            thin_pool_size.meta_size(),
            thin_pool_size.data_size(),
            thin_pool_size.mdv_size(),
        ]) {
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

        let backstore_device = backstore.device();

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
            segs_to_table(backstore_device, &meta_segments),
        )?;
        wipe_sectors(&meta_dev.devnode(), Sectors(0), meta_dev.size())?;

        let (dm_name, dm_uuid) = format_flex_ids(pool_uuid, FlexRole::ThinData);
        let data_dev = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            segs_to_table(backstore_device, &data_segments),
        )?;

        let (dm_name, dm_uuid) = format_flex_ids(pool_uuid, FlexRole::MetadataVolume);
        let mdv_dev = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            segs_to_table(backstore_device, &mdv_segments),
        )?;
        let mdv = MetadataVol::initialize(pool_uuid, mdv_dev)?;

        let (dm_name, dm_uuid) = format_thinpool_ids(pool_uuid, ThinPoolRole::Pool);
        let thinpool_dev = ThinPoolDev::new(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            meta_dev,
            data_dev,
            data_block_size,
            low_water_mark,
        )?;
        Ok(ThinPool {
            pool_uuid,
            thin_pool: thinpool_dev,
            meta_segments,
            meta_spare_segments: spare_segments,
            data_segments,
            mdv_segments,
            id_gen: ThinDevIdPool::new_from_ids(&[]),
            filesystems: Table::default(),
            mdv,
            backstore_device,
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
        data_block_size: Sectors,
        low_water_mark: DataBlocks,
        flex_devs: &FlexDevsSave,
        backstore: &Backstore,
    ) -> StratisResult<ThinPool> {
        let mdv_segments = flex_devs.meta_dev.to_vec();
        let meta_segments = flex_devs.thin_meta_dev.to_vec();
        let data_segments = flex_devs.thin_data_dev.to_vec();
        let spare_segments = flex_devs.thin_meta_dev_spare.to_vec();

        let backstore_device = backstore.device();

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

        let thinpool_dev = ThinPoolDev::setup(
            get_dm(),
            &thinpool_name,
            Some(&thinpool_uuid),
            meta_dev,
            data_dev,
            data_block_size,
            low_water_mark,
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

        // TODO: not fail completely if one filesystem setup fails?
        let filesystems = filesystem_metadatas
            .iter()
            .map(|fssave| {
                let (dm_name, dm_uuid) =
                    format_thin_ids(pool_uuid, ThinRole::Filesystem(fssave.uuid));
                let thin_dev = ThinDev::setup(
                    get_dm(),
                    &dm_name,
                    Some(&dm_uuid),
                    fssave.size,
                    &thinpool_dev,
                    fssave.thin_id,
                )?;
                Ok((
                    Name::new(fssave.name.to_owned()),
                    fssave.uuid,
                    StratFilesystem::setup(thin_dev),
                ))
            })
            .collect::<StratisResult<Vec<_>>>()?;

        let mut fs_table = Table::default();
        for (name, uuid, fs) in filesystems {
            let evicted = fs_table.insert(name, uuid, fs);
            if evicted.is_some() {
                let err_msg = "filesystems with duplicate UUID or name specified in metadata";
                return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg.into()));
            }
        }

        let thin_ids: Vec<ThinDevId> = filesystem_metadatas.iter().map(|x| x.thin_id).collect();
        Ok(ThinPool {
            pool_uuid,
            thin_pool: thinpool_dev,
            meta_segments,
            meta_spare_segments: spare_segments,
            data_segments,
            mdv_segments,
            id_gen: ThinDevIdPool::new_from_ids(&thin_ids),
            filesystems: fs_table,
            mdv,
            backstore_device,
        })
    }

    /// Run status checks and take actions on the thinpool and its components.
    /// Returns a bool communicating if a configuration change requiring a
    /// metadata save has been made.
    #[cfg(feature = "full_runtime")]
    pub fn check(&mut self, backstore: &mut Backstore) -> StratisResult<bool> {
        #![allow(match_same_arms)]
        assert_eq!(backstore.device(), self.backstore_device);

        let mut should_save: bool = false;

        let thinpool: dm::ThinPoolStatus = self.thin_pool.status(get_dm())?;
        match thinpool {
            dm::ThinPoolStatus::Working(ref status) => {
                match status.summary {
                    ThinPoolStatusSummary::Good => {}
                    ThinPoolStatusSummary::ReadOnly => {
                        // TODO: why is pool r/o and how do we get it
                        // rw again?
                    }
                    ThinPoolStatusSummary::OutOfSpace => {
                        // TODO: Add more space if possible, or
                        // prevent further usage
                        // Should never happen -- we should be extending first!
                    }
                }

                let usage = &status.usage;
                if usage.used_meta > cmp::max(usage.total_meta, META_LOWATER) - META_LOWATER {
                    // Request expansion of physical space allocated to the pool
                    // meta device.
                    // TODO: we just request that the space be doubled here.
                    // A more sophisticated approach might be in order.
                    let meta_extend_size = usage.total_meta;
                    match self.extend_thinpool_meta(meta_extend_size, backstore) {
                        #![allow(single_match)]
                        Ok(_) => should_save = true,
                        Err(_) => {} // TODO: Take pool offline?
                    }
                }

                if usage.used_data > cmp::max(usage.total_data, DATA_LOWATER) - DATA_LOWATER {
                    // Request expansion of physical space allocated to the pool
                    // TODO: we request that the space be doubled or use the remaining space by
                    // requesting the minimum total_data vs. available space.
                    // A more sophisticated approach might be in order.
                    match self.extend_thinpool(
                        DataBlocks(cmp::min(
                            *usage.total_data,
                            backstore.available() / DATA_BLOCK_SIZE,
                        )),
                        backstore,
                    ) {
                        #![allow(single_match)]
                        Ok(_) => should_save = true,
                        Err(_) => {} // TODO: Take pool offline?
                    }
                }
            }
            dm::ThinPoolStatus::Fail => {
                // TODO: Take pool offline?
                // TODO: Run thin_check
            }
        };

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

    /// Tear down the components managed here: filesystems, the MDV,
    /// and the actual thinpool device itself.
    pub fn teardown(self) -> StratisResult<()> {
        // Must succeed in tearing down all filesystems before the
        // thinpool..
        for (_, _, fs) in self.filesystems {
            fs.teardown()?;
        }
        self.thin_pool.teardown(get_dm())?;

        // ..but MDV has no DM dependencies with the above
        self.mdv.teardown()?;

        Ok(())
    }

    /// Expand the physical space allocated to a pool by extend_size.
    /// Return the number of DataBlocks added.
    // TODO: Refine this method. A hard fail if the request can not be
    // satisfied may not be correct.
    #[cfg(feature = "full_runtime")]
    fn extend_thinpool(
        &mut self,
        extend_size: DataBlocks,
        backstore: &mut Backstore,
    ) -> StratisResult<DataBlocks> {
        let backstore_device = self.backstore_device;
        assert_eq!(backstore.device(), backstore_device);
        if let Some(new_data_regions) = backstore.alloc_space(&[*extend_size * DATA_BLOCK_SIZE]) {
            self.suspend()?;
            self.extend_data(
                backstore_device,
                new_data_regions
                    .first()
                    .expect("len(new_data_regions) == 1"),
            )?;
            self.resume()?;
        } else {
            let err_msg = format!(
                "Insufficient space to accommodate request for {}",
                extend_size
            );
            return Err(StratisError::Engine(ErrorEnum::Error, err_msg));
        }
        Ok(extend_size)
    }

    /// Expand the physical space allocated to a pool meta by extend_size.
    /// Return the number of MetaBlocks added.
    #[cfg(feature = "full_runtime")]
    fn extend_thinpool_meta(
        &mut self,
        extend_size: MetaBlocks,
        backstore: &mut Backstore,
    ) -> StratisResult<MetaBlocks> {
        let backstore_device = self.backstore_device;
        assert_eq!(backstore.device(), backstore_device);
        if let Some(new_meta_regions) = backstore.alloc_space(&[extend_size.sectors()]) {
            self.suspend()?;
            self.extend_meta(
                backstore_device,
                new_meta_regions
                    .first()
                    .expect("len(new_meta_regions) == 1"),
            )?;
            self.resume()?;
        } else {
            let err_msg = format!(
                "Insufficient space to accommodate request for {}",
                extend_size
            );
            return Err(StratisError::Engine(ErrorEnum::Error, err_msg));
        }
        Ok(extend_size)
    }

    /// Extend the thinpool with new data regions.
    #[cfg(feature = "full_runtime")]
    fn extend_data(
        &mut self,
        device: Device,
        new_segs: &[(Sectors, Sectors)],
    ) -> StratisResult<()> {
        let segments = coalesce_segs(&self.data_segments, &new_segs.to_vec());
        self.thin_pool
            .set_data_table(get_dm(), segs_to_table(device, &segments))?;
        self.thin_pool.resume(get_dm())?;
        self.data_segments = segments;

        Ok(())
    }

    /// Extend the thinpool meta device with additional segments.
    #[cfg(feature = "full_runtime")]
    fn extend_meta(
        &mut self,
        device: Device,
        new_segs: &[(Sectors, Sectors)],
    ) -> StratisResult<()> {
        let segments = coalesce_segs(&self.meta_segments, &new_segs.to_vec());
        self.thin_pool
            .set_meta_table(get_dm(), segs_to_table(device, &segments))?;
        self.thin_pool.resume(get_dm())?;
        self.meta_segments = segments;

        Ok(())
    }

    /// The number of physical sectors in use, that is, unavailable for storage
    /// of additional user data, by this pool.
    // This includes all the sectors being held as spares for the meta device,
    // all the sectors allocated to the meta data device, and all the sectors
    // in use on the data device.
    pub fn total_physical_used(&self) -> StratisResult<Sectors> {
        let data_dev_used = match self.thin_pool.status(get_dm())? {
            dm::ThinPoolStatus::Working(ref status) => *status.usage.used_data * DATA_BLOCK_SIZE,
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

    #[cfg(feature = "full_runtime")]
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

    /// Create a filesystem within the thin pool. Given name must not
    /// already be in use.
    #[cfg(feature = "full_runtime")]
    pub fn create_filesystem(
        &mut self,
        pool_name: &str,
        name: &str,
        size: Option<Sectors>,
    ) -> StratisResult<FilesystemUuid> {
        let fs_uuid = Uuid::new_v4();
        let (dm_name, dm_uuid) = format_thin_ids(self.pool_uuid, ThinRole::Filesystem(fs_uuid));
        let thin_dev = ThinDev::new(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            size.unwrap_or(DEFAULT_THIN_DEV_SIZE),
            &self.thin_pool,
            self.id_gen.new_id()?,
        )?;

        let new_filesystem = StratFilesystem::initialize(fs_uuid, thin_dev)?;
        let name = Name::new(name.to_owned());
        self.mdv.save_fs(&name, fs_uuid, &new_filesystem)?;
        devlinks::filesystem_added(pool_name, &name, &new_filesystem.devnode())?;
        self.filesystems.insert(name, fs_uuid, new_filesystem);

        Ok(fs_uuid)
    }

    /// Create a filesystem snapshot of the origin.  Given origin_uuid
    /// must exist.  Returns the Uuid of the new filesystem.
    #[cfg(feature = "full_runtime")]
    pub fn snapshot_filesystem(
        &mut self,
        pool_name: &str,
        origin_uuid: FilesystemUuid,
        snapshot_name: &str,
    ) -> StratisResult<FilesystemUuid> {
        let snapshot_fs_uuid = Uuid::new_v4();
        let (snapshot_dm_name, snapshot_dm_uuid) =
            format_thin_ids(self.pool_uuid, ThinRole::Filesystem(snapshot_fs_uuid));
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
        Ok(snapshot_fs_uuid)
    }

    /// Destroy a filesystem within the thin pool.
    pub fn destroy_filesystem(
        &mut self,
        pool_name: &str,
        uuid: FilesystemUuid,
    ) -> StratisResult<()> {
        if let Some((fs_name, fs)) = self.filesystems.remove_by_uuid(uuid) {
            fs.destroy(&self.thin_pool)?;
            self.mdv.rm_fs(uuid)?;
            devlinks::filesystem_removed(pool_name, &fs_name)?;
        }
        Ok(())
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
            self.filesystems.insert(new_name.clone(), uuid, filesystem);
            devlinks::filesystem_renamed(pool_name, &old_name, &new_name)?;
            Ok(RenameAction::Renamed)
        }
    }

    /// The names of DM devices belonging to this pool that may generate events
    #[cfg(feature = "full_runtime")]
    pub fn get_eventing_dev_names(&self) -> Vec<DmNameBuf> {
        vec![
            format_flex_ids(self.pool_uuid, FlexRole::ThinMeta).0,
            format_flex_ids(self.pool_uuid, FlexRole::ThinData).0,
            format_flex_ids(self.pool_uuid, FlexRole::MetadataVolume).0,
            format_thinpool_ids(self.pool_uuid, ThinPoolRole::Pool).0,
        ]
    }

    /// Suspend the thinpool
    pub fn suspend(&mut self) -> StratisResult<()> {
        for (_, _, fs) in &mut self.filesystems {
            fs.suspend(false)?;
        }
        self.thin_pool.suspend(get_dm(), true)?;
        self.mdv.suspend()?;
        Ok(())
    }

    /// Resume the thinpool
    pub fn resume(&mut self) -> StratisResult<()> {
        self.mdv.resume()?;
        self.thin_pool.resume(get_dm())?;
        for (_, _, fs) in &mut self.filesystems {
            fs.resume()?;
        }
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
    meta_dev: LinearDev,
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
            DATA_LOWATER,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
        let fs_uuid = pool.create_filesystem(pool_name, "stratis_test_filesystem", None)
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
                    dm::ThinPoolStatus::Working(ref _status) => {
                        f.write_all(write_buf).unwrap();
                        if let Err(_e) = f.sync_data() {
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
                    "Expected full pool",
                );
            }
            dm::ThinPoolStatus::Fail => panic!("ThinPoolStatus::Fail  Expected working/full."),
        };
        // Add block devices to the pool and run check() to extend
        backstore
            .add_blockdevs(&remaining_paths, BlockDevTier::Data, true)
            .unwrap();
        pool.check(&mut backstore).unwrap();
        // Verify the pool is back in a Good state
        match pool.thin_pool.status(get_dm()).unwrap() {
            dm::ThinPoolStatus::Working(ref status) => {
                assert!(
                    status.summary == ThinPoolStatusSummary::Good,
                    "Expected pool to be restored to good state",
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
            DATA_LOWATER,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
        let fs_uuid = pool.create_filesystem(pool_name, "stratis_test_filesystem", None)
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
        pool.extend_thinpool(INITIAL_DATA_SIZE, &mut backstore)
            .unwrap();

        let snapshot_uuid = pool.snapshot_filesystem(pool_name, fs_uuid, "test_snapshot")
            .unwrap();
        let mut read_buf = [0u8; SECTOR_SIZE];
        let snapshot_tmp_dir = tempfile::Builder::new()
            .prefix("stratis_testing")
            .tempdir()
            .unwrap();
        {
            let (_, snapshot_filesystem) = pool.get_filesystem_by_uuid(snapshot_uuid).unwrap();
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
            DATA_LOWATER,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
        let fs_uuid = pool.create_filesystem(pool_name, &name1, None).unwrap();

        let action = pool.rename_filesystem(pool_name, fs_uuid, name2).unwrap();
        assert_eq!(action, RenameAction::Renamed);
        let flexdevs: FlexDevsSave = pool.record();
        pool.teardown().unwrap();

        let pool = ThinPool::setup(
            pool_uuid,
            DATA_BLOCK_SIZE,
            DATA_LOWATER,
            &flexdevs,
            &backstore,
        ).unwrap();

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
            DATA_LOWATER,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
        let fs_uuid = pool.create_filesystem(pool_name, "fsname", None).unwrap();

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

        let new_pool = ThinPool::setup(
            pool_uuid,
            DATA_BLOCK_SIZE,
            DATA_LOWATER,
            &pool.record(),
            &backstore,
        ).unwrap();

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
            DATA_LOWATER,
            &mut backstore,
        ).unwrap();
        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
        let fs_name = "stratis_test_filesystem";
        let fs_uuid = pool.create_filesystem(pool_name, &fs_name, None).unwrap();
        let thin_id = pool.get_filesystem_by_uuid(fs_uuid).unwrap().1.thin_id();
        let (dm_name, dm_uuid) = format_thin_ids(pool_uuid, ThinRole::Filesystem(fs_uuid));
        let thindev = ThinDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            DEFAULT_THIN_DEV_SIZE,
            &pool.thin_pool,
            thin_id,
        );
        assert!(thindev.is_ok());
        pool.destroy_filesystem(pool_name, fs_uuid).unwrap();

        let thindev = ThinDev::setup(
            get_dm(),
            &dm_name,
            None,
            DEFAULT_THIN_DEV_SIZE,
            &pool.thin_pool,
            thin_id,
        );
        assert!(thindev.is_err());
        let flexdevs: FlexDevsSave = pool.record();
        pool.teardown().unwrap();

        // Check that destroyed fs is not present in MDV. If the record
        // had been left on the MDV that didn't match a thin_id in the
        // thinpool, ::setup() will fail.
        let pool = ThinPool::setup(
            pool_uuid,
            DATA_BLOCK_SIZE,
            DATA_LOWATER,
            &flexdevs,
            &backstore,
        ).unwrap();

        assert!(pool.get_filesystem_by_uuid(fs_uuid).is_none());
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

    /// Verify that the meta device backing a ThinPool is expanded when meta
    /// utilization exceeds the META_LOWATER mark, by creating a ThinPool with
    /// a meta device smaller than the META_LOWATER.
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
            DATA_LOWATER,
            &mut backstore,
        ).unwrap();

        match thin_pool.thin_pool.status(get_dm()).unwrap() {
            dm::ThinPoolStatus::Working(ref status) => {
                let usage = &status.usage;
                assert_eq!(usage.total_meta, small_meta_size);
            }
            dm::ThinPoolStatus::Fail => panic!("thin_pool.status() failed"),
        }
        // The meta device is smaller than META_LOWATER, so it should be expanded
        // in the thin_pool.check() call.
        thin_pool.check(&mut backstore).unwrap();
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
            DATA_LOWATER,
            &mut backstore,
        ).unwrap();
        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
        let fs_name = "stratis_test_filesystem";
        let fs_uuid = pool.create_filesystem(pool_name, fs_name, None).unwrap();

        let devnode = pool.get_filesystem_by_uuid(fs_uuid).unwrap().1.devnode();
        // Braces to ensure f is closed before destroy
        {
            let mut f = OpenOptions::new().write(true).open(devnode).unwrap();
            // Write 1 more sector than is initially allocated to a pool
            let write_size = *INITIAL_DATA_SIZE * DATA_BLOCK_SIZE + Sectors(1);
            let buf = &[1u8; SECTOR_SIZE];
            for i in 0..*write_size {
                f.write_all(buf).unwrap();
                // Simulate handling a DM event by extending the pool when
                // the amount of free space in pool has decreased to the
                // DATA_LOWATER value.
                if i == *(*(INITIAL_DATA_SIZE - DATA_LOWATER) * DATA_BLOCK_SIZE) {
                    pool.extend_thinpool(INITIAL_DATA_SIZE, &mut backstore)
                        .unwrap();
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
            DATA_LOWATER,
            &mut backstore,
        ).unwrap();

        // Create a filesystem as small as possible.  Allocate 1 MiB bigger than
        // the low water mark.
        let fs_size = FILESYSTEM_LOWATER + Bytes(IEC::Mi).sectors();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
        let fs_name = "stratis_test_filesystem";
        let fs_uuid = pool.create_filesystem(pool_name, fs_name, Some(fs_size))
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
            DATA_LOWATER,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
        pool.create_filesystem(pool_name, "stratis_test_filesystem", None)
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
            DATA_LOWATER,
            &mut backstore,
        ).unwrap();

        let pool_name = "stratis_test_pool";
        devlinks::pool_added(&pool_name).unwrap();
        let fs_uuid = pool.create_filesystem(pool_name, "stratis_test_filesystem", None)
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
        let old_device = backstore.device();
        backstore
            .add_blockdevs(paths1, BlockDevTier::Cache, false)
            .unwrap();
        let new_device = backstore.device();
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

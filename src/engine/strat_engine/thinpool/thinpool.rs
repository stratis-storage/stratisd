// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Code to handle management of a pool's thinpool device.

use std::borrow::BorrowMut;
use std::process::Command;

use uuid::Uuid;

use devicemapper as dm;
use devicemapper::{DM, DataBlocks, DmDevice, DmName, DmNameBuf, IEC, LinearDev, MetaBlocks,
                   Sectors, Segment, ThinDev, ThinDevId, ThinPoolDev, ThinPoolStatusSummary,
                   device_exists};

use super::super::super::engine::{Filesystem, HasName};
use super::super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::super::structures::Table;
use super::super::super::types::{DevUuid, PoolUuid, FilesystemUuid, RenameAction};

use super::super::blockdevmgr::{BlockDevMgr, BlkDevSegment, map_to_dm};
use super::super::device::wipe_sectors;
use super::super::serde_structs::{FilesystemSave, FlexDevsSave, Recordable, ThinPoolDevSave};

use super::dmdevice::{FlexRole, ThinDevIdPool, ThinPoolRole, ThinRole, format_flex_name,
                      format_thinpool_name, format_thin_name};
use super::filesystem::{FilesystemStatus, StratFilesystem};
use super::mdv::MetadataVol;
use super::util::execute_cmd;

pub const DATA_BLOCK_SIZE: Sectors = Sectors(2048);
pub const DATA_LOWATER: DataBlocks = DataBlocks(512);
const META_LOWATER: MetaBlocks = MetaBlocks(512);

const DEFAULT_THIN_DEV_SIZE: Sectors = Sectors(2 * IEC::Gi); // 1 TiB

const INITIAL_META_SIZE: MetaBlocks = MetaBlocks(4096);
pub const INITIAL_DATA_SIZE: DataBlocks = DataBlocks(768);
const INITIAL_MDV_SIZE: Sectors = Sectors(32 * IEC::Ki); // 16 MiB


/// A ThinPool struct contains the thinpool itself, the spare
/// segments for its metadata device, and the filesystems and filesystem
/// metadata associated with it.
#[derive(Debug)]
pub struct ThinPool {
    pool_uuid: PoolUuid,
    thin_pool: ThinPoolDev,
    meta_segments: Vec<BlkDevSegment>,
    meta_spare_segments: Vec<BlkDevSegment>,
    data_segments: Vec<BlkDevSegment>,
    mdv_segments: Vec<BlkDevSegment>,
    id_gen: ThinDevIdPool,
    filesystems: Table<StratFilesystem>,
    mdv: MetadataVol,
}

impl ThinPool {
    /// Make a new thin pool.
    pub fn new(pool_uuid: PoolUuid,
               dm: &DM,
               data_block_size: Sectors,
               low_water_mark: DataBlocks,
               block_mgr: &mut BlockDevMgr)
               -> EngineResult<ThinPool> {
        let mut segments_list =
            match block_mgr.alloc_space(&[ThinPool::initial_metadata_size(),
                                          ThinPool::initial_metadata_size(),
                                          ThinPool::initial_data_size(),
                                          ThinPool::initial_mdv_size()]) {
                Some(sl) => sl,
                None => {
                    let err_msg = "Could not allocate sufficient space for thinpool devices.";
                    return Err(EngineError::Engine(ErrorEnum::Invalid, err_msg.into()));
                }
            };

        let mdv_segments = segments_list.pop().expect("len(segments_list) == 4");
        let data_segments = segments_list.pop().expect("len(segments_list) == 3");
        let spare_segments = segments_list.pop().expect("len(segments_list) == 2");
        let meta_segments = segments_list.pop().expect("len(segments_list) == 1");

        // When constructing a thin-pool, Stratis reserves the first N
        // sectors on a block device by creating a linear device with a
        // starting offset. DM writes the super block in the first block.
        // DM requires this first block to be zeros when the meta data for
        // the thin-pool is initially created. If we don't zero the
        // superblock DM issue error messages because it triggers code paths
        // that are trying to re-adopt the device with the attributes that
        // have been passed.
        let meta_dev = LinearDev::setup(dm,
                                        &format_flex_name(pool_uuid, FlexRole::ThinMeta),
                                        None,
                                        &map_to_dm(&meta_segments))?;
        wipe_sectors(&meta_dev.devnode(),
                     Sectors(0),
                     ThinPool::initial_metadata_size())?;

        let data_dev = LinearDev::setup(dm,
                                        &format_flex_name(pool_uuid, FlexRole::ThinData),
                                        None,
                                        &map_to_dm(&data_segments))?;

        let mdv_name = format_flex_name(pool_uuid, FlexRole::MetadataVolume);
        let mdv_dev = LinearDev::setup(dm, &mdv_name, None, &map_to_dm(&mdv_segments))?;
        let mdv = MetadataVol::initialize(pool_uuid, mdv_dev)?;

        let name = format_thinpool_name(pool_uuid, ThinPoolRole::Pool);
        let thinpool_dev = ThinPoolDev::new(dm,
                                            name.as_ref(),
                                            None,
                                            meta_dev,
                                            data_dev,
                                            data_block_size,
                                            low_water_mark)?;
        Ok(ThinPool {
               pool_uuid: pool_uuid,
               thin_pool: thinpool_dev,
               meta_segments: meta_segments,
               meta_spare_segments: spare_segments,
               data_segments: data_segments,
               mdv_segments: mdv_segments,
               id_gen: ThinDevIdPool::new_from_ids(&[]),
               filesystems: Table::default(),
               mdv: mdv,
           })
    }

    /// Set up an "existing" thin pool.
    /// A thin pool must store the metadata for its thin devices, regardless of
    /// whether it has an existing device node. An existing thin pool device
    /// is a device where the metadata is already stored on its meta device.
    /// If initial setup fails due to a thin_check failure, attempt to fix
    /// the problem by running thin_repair. If failure recurs, return an
    /// error.
    pub fn setup(pool_uuid: PoolUuid,
                 dm: &DM,
                 data_block_size: Sectors,
                 low_water_mark: DataBlocks,
                 flex_devs: &FlexDevsSave,
                 bd_mgr: &BlockDevMgr)
                 -> EngineResult<ThinPool> {
        let uuid_to_devno = bd_mgr.uuid_to_devno();
        let mapper = |triple: &(DevUuid, Sectors, Sectors)| -> EngineResult<BlkDevSegment> {
            let device = uuid_to_devno(triple.0)
                .ok_or_else(|| {
                                EngineError::Engine(ErrorEnum::NotFound,
                                                    format!("missing device for UUID {:?}",
                                                            &triple.0))
                            })?;
            Ok(BlkDevSegment::new(triple.0, Segment::new(device, triple.1, triple.2)))
        };

        let mdv_segments = flex_devs
            .meta_dev
            .iter()
            .map(&mapper)
            .collect::<EngineResult<Vec<_>>>()?;

        let meta_segments = flex_devs
            .thin_meta_dev
            .iter()
            .map(&mapper)
            .collect::<EngineResult<Vec<_>>>()?;

        let data_segments = flex_devs
            .thin_data_dev
            .iter()
            .map(&mapper)
            .collect::<EngineResult<Vec<_>>>()?;

        let spare_segments = flex_devs
            .thin_meta_dev_spare
            .iter()
            .map(&mapper)
            .collect::<EngineResult<Vec<_>>>()?;

        let thinpool_name = format_thinpool_name(pool_uuid, ThinPoolRole::Pool);
        let (meta_dev, meta_segments, spare_segments) =
            setup_metadev(dm, pool_uuid, &thinpool_name, meta_segments, spare_segments)?;

        let data_dev = LinearDev::setup(dm,
                                        &format_flex_name(pool_uuid, FlexRole::ThinData),
                                        None,
                                        &map_to_dm(&data_segments))?;

        let thinpool_dev = ThinPoolDev::setup(dm,
                                              &thinpool_name,
                                              None,
                                              meta_dev,
                                              data_dev,
                                              data_block_size,
                                              low_water_mark)?;

        let mdv_dev = LinearDev::setup(dm,
                                       &format_flex_name(pool_uuid, FlexRole::MetadataVolume),
                                       None,
                                       &map_to_dm(&mdv_segments))?;
        let mdv = MetadataVol::setup(pool_uuid, mdv_dev)?;
        let filesystem_metadatas = mdv.filesystems()?;

        // TODO: not fail completely if one filesystem setup fails?
        let filesystems = {
            // Set up a filesystem from its metadata.
            let get_filesystem = |fssave: &FilesystemSave| -> EngineResult<StratFilesystem> {
                let device_name = format_thin_name(pool_uuid, ThinRole::Filesystem(fssave.uuid));
                let thin_dev = ThinDev::setup(dm,
                                              device_name.as_ref(),
                                              None,
                                              fssave.size,
                                              &thinpool_dev,
                                              fssave.thin_id)?;
                Ok(StratFilesystem::setup(fssave.uuid, &fssave.name, thin_dev))
            };

            filesystem_metadatas
                .iter()
                .map(get_filesystem)
                .collect::<EngineResult<Vec<_>>>()?
        };

        let mut fs_table = Table::default();
        for fs in filesystems {
            let evicted = fs_table.insert(fs);
            if !evicted.is_empty() {
                let err_msg = "filesystems with duplicate UUID or name specified in metadata";
                return Err(EngineError::Engine(ErrorEnum::Invalid, err_msg.into()));
            }
        }

        let thin_ids: Vec<ThinDevId> = filesystem_metadatas.iter().map(|x| x.thin_id).collect();
        Ok(ThinPool {
               pool_uuid: pool_uuid,
               thin_pool: thinpool_dev,
               meta_segments: meta_segments,
               meta_spare_segments: spare_segments,
               data_segments: data_segments,
               mdv_segments: mdv_segments,
               id_gen: ThinDevIdPool::new_from_ids(&thin_ids),
               filesystems: fs_table,
               mdv: mdv,
           })
    }

    /// Initial size for a pool's meta data device.
    fn initial_metadata_size() -> Sectors {
        INITIAL_META_SIZE.sectors()
    }

    /// Initial size for a pool's data device.
    fn initial_data_size() -> Sectors {
        *INITIAL_DATA_SIZE * DATA_BLOCK_SIZE
    }

    /// Initial size for a pool's filesystem metadata volume.
    fn initial_mdv_size() -> Sectors {
        INITIAL_MDV_SIZE
    }

    /// Run status checks and take actions on the thinpool and its components.
    pub fn check(&mut self, dm: &DM, bd_mgr: &mut BlockDevMgr) -> EngineResult<()> {
        #![allow(match_same_arms)]
        let thinpool: dm::ThinPoolStatus = self.thin_pool.status(dm)?;
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
                if usage.used_meta > usage.total_meta - META_LOWATER {
                    // TODO: Extend meta device
                }

                if usage.used_data > usage.total_data - DATA_LOWATER {
                    // Request expansion of physical space allocated to the pool
                    // TODO: we just request that the space be doubled here.
                    // A more sophisticated approach might be in order.
                    match self.extend_thinpool(dm, usage.total_data, bd_mgr) {
                        #![allow(single_match)]
                        Ok(_) => {}
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
            .into_iter()
            .map(|fs| fs.check(dm))
            .collect::<EngineResult<Vec<_>>>()?;

        for fs_status in filesystems {
            if let FilesystemStatus::Failed = fs_status {
                // TODO: filesystem failed, how to recover?
            }
        }
        Ok(())
    }

    /// Tear down the components managed here: filesystems, the MDV,
    /// and the actual thinpool device itself.
    pub fn teardown(self, dm: &DM) -> EngineResult<()> {
        // Must succeed in tearing down all filesystems before the
        // thinpool..
        for fs in self.filesystems.empty() {
            fs.teardown(dm)?;
        }
        self.thin_pool.teardown(dm)?;

        // ..but MDV has no DM dependencies with the above
        self.mdv.teardown(dm)?;

        Ok(())
    }

    /// Expand the physical space allocated to a pool by extend_size.
    /// Return the number of DataBlocks added.
    // TODO: Refine this method. A hard fail if the request can not be
    // satisfied may not be correct.
    fn extend_thinpool(&mut self,
                       dm: &DM,
                       extend_size: DataBlocks,
                       bd_mgr: &mut BlockDevMgr)
                       -> EngineResult<DataBlocks> {
        if let Some(mut new_data_regions) = bd_mgr.alloc_space(&[*extend_size * DATA_BLOCK_SIZE]) {
            self.extend_data(dm,
                             &new_data_regions
                                  .pop()
                                  .expect("len(new_data_regions) == 1"))?;
        } else {
            let err_msg = format!("Insufficient space to accomodate request for {}",
                                  extend_size);
            return Err(EngineError::Engine(ErrorEnum::Error, err_msg));
        }
        Ok(extend_size)
    }

    /// Extend the thinpool with new data regions.
    fn extend_data(&mut self, dm: &DM, new_segs: &[BlkDevSegment]) -> EngineResult<()> {
        let mut segments = Vec::with_capacity(self.data_segments.len() + new_segs.len());
        segments.extend_from_slice(&self.data_segments);

        // Last existing and first new may be contiguous. Coalesce into
        // a single BlkDevSegment if so.
        let coalesced_new_first = {
            match new_segs.first() {
                Some(new_first) => {
                    let old_last = segments
                        .last_mut()
                        .expect("thin pool must always have some data segments");
                    if old_last.uuid == new_first.uuid &&
                       (old_last.segment.start + old_last.segment.length ==
                        new_first.segment.start) {
                        old_last.segment.length += new_first.segment.length;
                        true
                    } else {
                        false
                    }
                }
                None => false,
            }
        };

        if coalesced_new_first {
            segments.extend_from_slice(&new_segs[1..]);
        } else {
            segments.extend_from_slice(new_segs);
        }

        self.thin_pool
            .set_data_segments(dm, &map_to_dm(&segments))?;
        self.data_segments = segments;

        Ok(())
    }

    /// The number of physical sectors in use, that is, unavailable for storage
    /// of additional user data, by this pool.
    // This includes all the sectors being held as spares for the meta device,
    // all the sectors allocated to the meta data device, and all the sectors
    // in use on the data device.
    pub fn total_physical_used(&self) -> EngineResult<Sectors> {
        let data_dev_used = match self.thin_pool.status(&DM::new()?)? {
            dm::ThinPoolStatus::Working(ref status) => *status.usage.used_data * DATA_BLOCK_SIZE,
            _ => {
                let err_msg = "thin pool failed, could not obtain usage";
                return Err(EngineError::Engine(ErrorEnum::Invalid, err_msg.into()));
            }
        };

        let spare_total = self.meta_spare_segments
            .iter()
            .map(|s| s.segment.length)
            .sum();
        let meta_dev_total = self.thin_pool
            .meta_dev()
            .segments()
            .iter()
            .map(|s| s.length)
            .sum();

        let mdv_total = self.mdv_segments
            .iter()
            .map(|s| s.segment.length)
            .sum();

        Ok(data_dev_used + spare_total + meta_dev_total + mdv_total)
    }

    pub fn get_filesystem_by_uuid(&self, uuid: FilesystemUuid) -> Option<&StratFilesystem> {
        self.filesystems.get_by_uuid(uuid)
    }

    pub fn get_mut_filesystem_by_uuid(&mut self,
                                      uuid: FilesystemUuid)
                                      -> Option<&mut StratFilesystem> {
        self.filesystems.get_mut_by_uuid(uuid)
    }

    #[allow(dead_code)]
    pub fn get_filesystem_by_name(&self, name: &str) -> Option<&StratFilesystem> {
        self.filesystems.get_by_name(name)
    }

    pub fn get_mut_filesystem_by_name(&mut self, name: &str) -> Option<&mut StratFilesystem> {
        self.filesystems.get_mut_by_name(name)
    }

    pub fn has_filesystems(&self) -> bool {
        !self.filesystems.is_empty()
    }

    pub fn filesystems(&self) -> Vec<&Filesystem> {
        self.filesystems
            .into_iter()
            .map(|x| x as &Filesystem)
            .collect()
    }

    /// Create a filesystem within the thin pool. Given name must not
    /// already be in use.
    pub fn create_filesystem(&mut self,
                             name: &str,
                             dm: &DM,
                             size: Option<Sectors>)
                             -> EngineResult<FilesystemUuid> {
        let fs_uuid = Uuid::new_v4();
        let device_name = format_thin_name(self.pool_uuid, ThinRole::Filesystem(fs_uuid));
        let thin_dev = ThinDev::new(dm,
                                    device_name.as_ref(),
                                    None,
                                    size.unwrap_or(DEFAULT_THIN_DEV_SIZE),
                                    &self.thin_pool,
                                    self.id_gen.new_id()?)?;

        let new_filesystem = StratFilesystem::initialize(fs_uuid, name, thin_dev)?;
        self.mdv.save_fs(&new_filesystem)?;
        self.filesystems.insert(new_filesystem);

        Ok(fs_uuid)
    }

    /// Create a filesystem snapshot of the origin.  Given origin_uuid
    /// must exist.  Returns the Uuid of the new filesystem.
    pub fn snapshot_filesystem(&mut self,
                               dm: &DM,
                               origin_uuid: FilesystemUuid,
                               snapshot_name: &str)
                               -> EngineResult<FilesystemUuid> {
        let snapshot_fs_uuid = Uuid::new_v4();
        let snapshot_dmname = format_thin_name(self.pool_uuid,
                                               ThinRole::Filesystem(snapshot_fs_uuid));
        let snapshot_id = self.id_gen.new_id()?;
        let new_filesystem = match self.get_filesystem_by_uuid(origin_uuid) {
            Some(filesystem) => {
                filesystem
                    .snapshot(dm,
                              &self.thin_pool,
                              snapshot_name,
                              snapshot_dmname.as_ref(),
                              snapshot_fs_uuid,
                              snapshot_id)?
            }
            None => {
                return Err(EngineError::Engine(ErrorEnum::Error,
                                               "snapshot_filesystem failed, filesystem not found"
                                                   .into()));
            }
        };
        self.mdv.save_fs(&new_filesystem)?;
        self.filesystems.insert(new_filesystem);
        Ok(snapshot_fs_uuid)
    }

    /// Destroy a filesystem within the thin pool.
    pub fn destroy_filesystem(&mut self, dm: &DM, uuid: FilesystemUuid) -> EngineResult<()> {
        if let Some(fs) = self.filesystems.remove_by_uuid(uuid) {
            fs.destroy(dm, &self.thin_pool)?;
            self.mdv.rm_fs(uuid)?;
        }
        Ok(())
    }

    /// Rename a filesystem within the thin pool.
    pub fn rename_filesystem(&mut self,
                             uuid: FilesystemUuid,
                             new_name: &str)
                             -> EngineResult<RenameAction> {

        let old_name = rename_filesystem_pre!(self; uuid; new_name);

        let mut filesystem =
            self.filesystems
                .remove_by_uuid(uuid)
                .expect("Must succeed since self.filesystems.get_by_uuid() returned a value");

        filesystem.rename(new_name);
        if let Err(err) = self.mdv.save_fs(&filesystem) {
            filesystem.rename(&old_name);
            self.filesystems.insert(filesystem);
            Err(err)
        } else {
            self.filesystems.insert(filesystem);
            Ok(RenameAction::Renamed)
        }
    }

    /// The names of DM devices belonging to this pool that may generate events
    pub fn get_eventing_dev_names(&self) -> Vec<DmNameBuf> {
        vec![format_flex_name(self.pool_uuid, FlexRole::ThinMeta),
             format_flex_name(self.pool_uuid, FlexRole::ThinData),
             format_flex_name(self.pool_uuid, FlexRole::MetadataVolume),
             format_thinpool_name(self.pool_uuid, ThinPoolRole::Pool)]

    }
}

impl Recordable<FlexDevsSave> for ThinPool {
    fn record(&self) -> FlexDevsSave {
        FlexDevsSave {
            meta_dev: self.mdv_segments.record(),
            thin_meta_dev: self.meta_segments.record(),
            thin_data_dev: self.data_segments.record(),
            thin_meta_dev_spare: self.meta_spare_segments.record(),
        }
    }
}

impl Recordable<ThinPoolDevSave> for ThinPool {
    fn record(&self) -> ThinPoolDevSave {
        ThinPoolDevSave { data_block_size: self.thin_pool.data_block_size() }
    }
}

/// Setup metadata dev for thinpool.
/// Attempt to verify that the metadata dev is valid for the given thinpool
/// using thin_check. If thin_check indicates that the metadata is corrupted
/// run thin_repair, using the spare segments, to try to repair the metadata
/// dev. Return the metadata device, the metadata segments, and the
/// spare segments.
fn setup_metadev(dm: &DM,
                 pool_uuid: PoolUuid,
                 thinpool_name: &DmName,
                 meta_segments: Vec<BlkDevSegment>,
                 spare_segments: Vec<BlkDevSegment>)
                 -> EngineResult<(LinearDev, Vec<BlkDevSegment>, Vec<BlkDevSegment>)> {
    #![allow(collapsible_if)]
    let mut meta_dev = LinearDev::setup(dm,
                                        &format_flex_name(pool_uuid, FlexRole::ThinMeta),
                                        None,
                                        &map_to_dm(&meta_segments))?;

    if !device_exists(dm, thinpool_name)? {
        // TODO: Refine policy about failure to run thin_check.
        // If, e.g., thin_check is unavailable, that doesn't necessarily
        // mean that data is corrupted.
        if !execute_cmd(Command::new("thin_check")
                            .arg("-q")
                            .arg(&meta_dev.devnode()),
                        &format!("thin_check failed for pool {}", thinpool_name))
                    .is_ok() {
            meta_dev = attempt_thin_repair(pool_uuid, dm, meta_dev, &spare_segments)?;
            return Ok((meta_dev, spare_segments, meta_segments));
        }
    }

    Ok((meta_dev, meta_segments, spare_segments))
}

/// Attempt a thin repair operation on the meta device.
/// If the operation succeeds, teardown the old meta device,
/// and return the new meta device.
fn attempt_thin_repair(pool_uuid: PoolUuid,
                       dm: &DM,
                       meta_dev: LinearDev,
                       spare_segments: &[BlkDevSegment])
                       -> EngineResult<LinearDev> {
    let mut new_meta_dev = LinearDev::setup(dm,
                                            &format_flex_name(pool_uuid, FlexRole::ThinMetaSpare),
                                            None,
                                            &map_to_dm(spare_segments))?;

    execute_cmd(Command::new("thin_repair")
                    .arg("-i")
                    .arg(&meta_dev.devnode())
                    .arg("-o")
                    .arg(&new_meta_dev.devnode()),
                &format!("thin_repair failed, pool ({:?}) unusable", pool_uuid))?;

    let name = meta_dev.name().to_owned();
    meta_dev.teardown(dm)?;
    new_meta_dev.set_name(dm, name.as_ref())?;

    Ok(new_meta_dev)
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::io::{Read, Write};
    use std::path::Path;

    use nix::mount::{MsFlags, mount, umount};
    use uuid::Uuid;

    use devicemapper::{Bytes, SECTOR_SIZE};

    use super::super::super::metadata::MIN_MDA_SECTORS;
    use super::super::super::tests::{loopbacked, real};
    use super::super::super::tests::tempdir::TempDir;

    use super::super::filesystem::{FILESYSTEM_LOWATER, fs_usage};

    use super::*;

    /// Verify a snapshot has the same files and same contents as the origin.
    fn test_filesystem_snapshot(paths: &[&Path]) {
        let pool_uuid = Uuid::new_v4();
        let dm = DM::new().unwrap();
        let mut mgr = BlockDevMgr::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).unwrap();
        let mut pool = ThinPool::new(pool_uuid, &dm, DATA_BLOCK_SIZE, DATA_LOWATER, &mut mgr)
            .unwrap();

        let fs_uuid = pool.create_filesystem("stratis_test_filesystem", &dm, None)
            .unwrap();

        let write_buf = &[8u8; SECTOR_SIZE];
        let file_count = 10;

        let source_tmp_dir = TempDir::new("stratis_testing").unwrap();
        {
            // to allow mutable borrow of pool
            let filesystem = pool.get_filesystem_by_uuid(fs_uuid).unwrap();
            mount(Some(&filesystem.devnode()),
                  source_tmp_dir.path(),
                  Some("xfs"),
                  MsFlags::empty(),
                  None as Option<&str>)
                    .unwrap();
            for i in 0..file_count {
                let file_path = source_tmp_dir
                    .path()
                    .join(format!("stratis_test{}.txt", i));
                let mut f = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(file_path)
                    .unwrap();
                f.write_all(write_buf).unwrap();
                f.flush().unwrap();
            }
        }

        // Double the size of the data device. The space initially allocated
        // to a pool is close to consumed by the filesystem and few files
        // written above. If we attempt to update the UUID on the snapshot
        // without expanding the pool, the pool will go into out-of-data-space
        // (queue IO) mode, causing the test to fail.
        pool.extend_thinpool(&dm, INITIAL_DATA_SIZE, &mut mgr)
            .unwrap();

        let snapshot_uuid = pool.snapshot_filesystem(&dm, fs_uuid, "test_snapshot")
            .unwrap();
        let mut read_buf = [0u8; SECTOR_SIZE];
        let snapshot_tmp_dir = TempDir::new("stratis_testing").unwrap();
        {
            let snapshot_filesystem = pool.get_filesystem_by_uuid(snapshot_uuid).unwrap();
            mount(Some(&snapshot_filesystem.devnode()),
                  snapshot_tmp_dir.path(),
                  Some("xfs"),
                  MsFlags::empty(),
                  None as Option<&str>)
                    .unwrap();
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
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(2, 3),
                                   test_filesystem_snapshot);
    }

    #[test]
    pub fn real_test_filesystem_snapshot() {
        real::test_with_spec(real::DeviceLimits::AtLeast(2), test_filesystem_snapshot);
    }

    /// Verify that a filesystem rename causes the filesystem metadata to be
    /// updated.
    fn test_filesystem_rename(paths: &[&Path]) {
        let name1 = "name1";
        let name2 = "name2";

        let pool_uuid = Uuid::new_v4();
        let dm = DM::new().unwrap();
        let mut mgr = BlockDevMgr::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).unwrap();
        let mut pool = ThinPool::new(pool_uuid, &dm, DATA_BLOCK_SIZE, DATA_LOWATER, &mut mgr)
            .unwrap();

        let fs_uuid = pool.create_filesystem(&name1, &dm, None).unwrap();

        let action = pool.rename_filesystem(fs_uuid, name2).unwrap();
        assert_eq!(action, RenameAction::Renamed);
        let flexdevs: FlexDevsSave = pool.record();
        pool.teardown(&dm).unwrap();

        let pool = ThinPool::setup(pool_uuid,
                                   &dm,
                                   DATA_BLOCK_SIZE,
                                   DATA_LOWATER,
                                   &flexdevs,
                                   &mgr)
                .unwrap();

        assert_eq!(pool.get_filesystem_by_uuid(fs_uuid).unwrap().name(), name2);
    }

    #[test]
    pub fn loop_test_filesystem_rename() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(1, 3),
                                   test_filesystem_rename);
    }

    #[test]
    pub fn real_test_filesystem_rename() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1), test_filesystem_rename);
    }

    /// Verify that setting up a pool when the pool has not been previously torn
    /// down does not fail. Clutter the original pool with a filesystem with
    /// some data on it.
    fn test_pool_setup(paths: &[&Path]) {
        let pool_uuid = Uuid::new_v4();
        let dm = DM::new().unwrap();
        let mut mgr = BlockDevMgr::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).unwrap();
        let mut pool = ThinPool::new(pool_uuid, &dm, DATA_BLOCK_SIZE, DATA_LOWATER, &mut mgr)
            .unwrap();

        let fs_uuid = pool.create_filesystem("fsname", &dm, None).unwrap();

        let tmp_dir = TempDir::new("stratis_testing").unwrap();
        let new_file = tmp_dir.path().join("stratis_test.txt");
        {
            let fs = pool.get_filesystem_by_uuid(fs_uuid).unwrap();
            mount(Some(&fs.devnode()),
                  tmp_dir.path(),
                  Some("xfs"),
                  MsFlags::empty(),
                  None as Option<&str>)
                    .unwrap();
            writeln!(&OpenOptions::new()
                          .create(true)
                          .write(true)
                          .open(new_file)
                          .unwrap(),
                     "data")
                    .unwrap();
        }

        let new_pool = ThinPool::setup(pool_uuid,
                                       &dm,
                                       DATA_BLOCK_SIZE,
                                       DATA_LOWATER,
                                       &pool.record(),
                                       &mgr)
                .unwrap();

        assert!(new_pool.get_filesystem_by_uuid(fs_uuid).is_some());
    }

    #[test]
    pub fn loop_test_pool_setup() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(1, 3), test_pool_setup);
    }

    #[test]
    pub fn real_test_pool_setup() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1), test_pool_setup);
    }
    /// Verify that destroy_filesystems actually deallocates the space
    /// from the thinpool, by attempting to reinstantiate it using the
    /// same thin id and verifying that it fails.
    fn test_thindev_destroy(paths: &[&Path]) -> () {
        let pool_uuid = Uuid::new_v4();
        let dm = DM::new().unwrap();
        let mut mgr = BlockDevMgr::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).unwrap();
        let mut pool = ThinPool::new(pool_uuid, &dm, DATA_BLOCK_SIZE, DATA_LOWATER, &mut mgr)
            .unwrap();
        let fs_name = "stratis_test_filesystem";
        let fs_uuid = pool.create_filesystem(&fs_name, &dm, None).unwrap();
        let thin_id = pool.get_filesystem_by_uuid(fs_uuid).unwrap().thin_id();
        let device_name = format_thin_name(pool_uuid, ThinRole::Filesystem(fs_uuid));
        let thindev = ThinDev::setup(&dm,
                                     device_name.as_ref(),
                                     None,
                                     DEFAULT_THIN_DEV_SIZE,
                                     &pool.thin_pool,
                                     thin_id);
        assert!(thindev.is_ok());
        pool.destroy_filesystem(&dm, fs_uuid).unwrap();

        let thindev = ThinDev::setup(&dm,
                                     device_name.as_ref(),
                                     None,
                                     DEFAULT_THIN_DEV_SIZE,
                                     &pool.thin_pool,
                                     thin_id);
        assert!(thindev.is_err());
        let flexdevs: FlexDevsSave = pool.record();
        pool.teardown(&dm).unwrap();

        // Check that destroyed fs is not present in MDV. If the record
        // had been left on the MDV that didn't match a thin_id in the
        // thinpool, ::setup() will fail.
        let pool = ThinPool::setup(pool_uuid,
                                   &dm,
                                   DATA_BLOCK_SIZE,
                                   DATA_LOWATER,
                                   &flexdevs,
                                   &mgr)
                .unwrap();

        assert!(pool.get_filesystem_by_uuid(fs_uuid).is_none());
    }

    #[test]
    pub fn loop_test_thindev_destroy() {
        // This test requires more than 1 GiB.
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(2, 3), test_thindev_destroy);
    }

    #[test]
    pub fn real_test_thindev_destroy() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1), test_thindev_destroy);
    }

    /// Verify that the physical space allocated to a pool is expanded when
    /// the number of sectors written to a thin-dev in the pool exceeds the
    /// INITIAL_DATA_SIZE.  If we are able to write more sectors to the
    /// filesystem than are initially allocated to the pool, the pool must
    /// have been expanded.
    fn test_thinpool_expand(paths: &[&Path]) -> () {
        let pool_uuid = Uuid::new_v4();
        let dm = DM::new().unwrap();
        let mut mgr = BlockDevMgr::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).unwrap();
        let mut pool = ThinPool::new(pool_uuid, &dm, DATA_BLOCK_SIZE, DATA_LOWATER, &mut mgr)
            .unwrap();
        let fs_name = "stratis_test_filesystem";
        let fs_uuid = pool.create_filesystem(&fs_name, &dm, None).unwrap();

        let devnode = pool.get_filesystem_by_uuid(fs_uuid).unwrap().devnode();
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
                    pool.extend_thinpool(&dm, INITIAL_DATA_SIZE, &mut mgr)
                        .unwrap();
                }
            }
        }
    }

    #[test]
    pub fn loop_test_thinpool_expand() {
        // This test requires more than 1 GiB.
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(2, 3), test_thinpool_expand);
    }

    #[test]
    pub fn real_test_thinpool_expand() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1), test_thinpool_expand);
    }

    /// Verify that the logical space allocated to a filesystem is expanded when
    /// the number of sectors written to the filesystem causes the free space to
    /// dip below the FILESYSTEM_LOWATER mark. Verify that the space has been
    /// expanded by calling filesystem.check() then looking at the total space
    /// compared to the original size.
    fn test_xfs_expand(paths: &[&Path]) -> () {
        let pool_uuid = Uuid::new_v4();
        let dm = DM::new().unwrap();
        let mut mgr = BlockDevMgr::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).unwrap();
        let mut pool = ThinPool::new(pool_uuid, &dm, DATA_BLOCK_SIZE, DATA_LOWATER, &mut mgr)
            .unwrap();

        // Create a filesytem as small as possible.  Allocate 1 MiB bigger than
        // the low water mark.
        let fs_size = FILESYSTEM_LOWATER + Bytes(IEC::Mi).sectors();

        let fs_name = "stratis_test_filesystem";
        let fs_uuid = pool.create_filesystem(&fs_name, &dm, Some(fs_size))
            .unwrap();

        // Braces to ensure f is closed before destroy and the borrow of
        // pool is complete
        {
            let filesystem = pool.get_mut_filesystem_by_uuid(fs_uuid).unwrap();
            // Write 2 MiB of data. The filesystem's free space is now 1 MiB
            // below FILESYSTEM_LOWATER.
            let write_size = Bytes(IEC::Mi * 2).sectors();
            let tmp_dir = TempDir::new("stratis_testing").unwrap();
            mount(Some(&filesystem.devnode()),
                  tmp_dir.path(),
                  Some("xfs"),
                  MsFlags::empty(),
                  None as Option<&str>)
                    .unwrap();
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
            filesystem.check(&dm).unwrap();
            let (fs_total_bytes, _) = fs_usage(&tmp_dir.path()).unwrap();
            assert!(fs_total_bytes > orig_fs_total_bytes);
            umount(tmp_dir.path()).unwrap();
        }
    }

    #[test]
    pub fn loop_test_xfs_expand() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(1, 3), test_xfs_expand);
    }

    #[test]
    pub fn real_test_xfs_expand() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1), test_xfs_expand);
    }
}

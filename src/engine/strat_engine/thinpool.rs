// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Code to handle management of a pool's thinpool device.

use std::borrow::BorrowMut;
use std::process::Command;

use uuid::Uuid;

use devicemapper as dm;
use devicemapper::{DM, DataBlocks, DmError, LinearDev, MetaBlocks, Sectors, Segment, ThinDev,
                   ThinDevId, ThinPoolDev};
use devicemapper::ErrorEnum::CheckFailed;

use super::super::consts::IEC;
use super::super::engine::{Filesystem, HasName};
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::structures::Table;
use super::super::types::{PoolUuid, FilesystemUuid, RenameAction};

use super::blockdevmgr::BlockDevMgr;
use super::device::wipe_sectors;
use super::dmdevice::{FlexRole, ThinDevIdPool, ThinPoolRole, ThinRole, format_flex_name,
                      format_thinpool_name, format_thin_name};
use super::filesystem::{StratFilesystem, FilesystemStatus};
use super::mdv::MetadataVol;
use super::serde_structs::{FilesystemSave, FlexDevsSave, Recordable, ThinPoolDevSave};


pub const DATA_BLOCK_SIZE: Sectors = Sectors(2048);
pub const DATA_LOWATER: DataBlocks = DataBlocks(512);
pub const META_LOWATER: MetaBlocks = MetaBlocks(512);

const DEFAULT_THIN_DEV_SIZE: Sectors = Sectors(2 * IEC::Gi); // 1 TiB

const INITIAL_META_SIZE: MetaBlocks = MetaBlocks(4096);
pub const INITIAL_DATA_SIZE: DataBlocks = DataBlocks(768);
const INITIAL_MDV_SIZE: Sectors = Sectors(32 * IEC::Ki); // 16 MiB


/// A ThinPool struct contains the thinpool itself, the spare
/// segments for its metadata device, and the filesystems and filesystem
/// metadata associated with it.
#[derive(Debug)]
pub struct ThinPool {
    thin_pool: ThinPoolDev,
    meta_spare: Vec<Segment>,
    id_gen: ThinDevIdPool,
    filesystems: Table<StratFilesystem>,
    mdv: MetadataVol,
}

/// A struct returning the status of the distinct parts of the
/// thinpool.
pub struct ThinPoolStatus {
    /// The status of the thinpool itself.
    pub thinpool: dm::ThinPoolStatus,
    /// The status of the filesystems within the thinpool.
    pub filesystems: Vec<FilesystemStatus>,
}

impl ThinPool {
    /// Make a new thin pool.
    /// Precondition: block_mgr can allocate the space required.
    pub fn new(pool_uuid: PoolUuid,
               dm: &DM,
               data_block_size: Sectors,
               low_water_mark: DataBlocks,
               block_mgr: &mut BlockDevMgr)
               -> EngineResult<ThinPool> {
        assert!(block_mgr.avail_space() >= ThinPool::initial_size());

        let meta_segments = block_mgr
            .alloc_space(ThinPool::initial_metadata_size())
            .expect("blockmgr must not fail, already checked for space");

        let spare_segments = block_mgr
            .alloc_space(ThinPool::initial_metadata_size())
            .expect("blockmgr must not fail, already checked for space");

        let data_segments = block_mgr
            .alloc_space(ThinPool::initial_data_size())
            .expect("blockmgr must not fail, already checked for space");

        let mdv_segments = block_mgr
            .alloc_space(ThinPool::initial_mdv_size())
            .expect("blockmgr must not fail, already checked for space");

        // When constructing a thin-pool, Stratis reserves the first N
        // sectors on a block device by creating a linear device with a
        // starting offset. DM writes the super block in the first block.
        // DM requires this first block to be zeros when the meta data for
        // the thin-pool is initially created. If we don't zero the
        // superblock DM issue error messages because it triggers code paths
        // that are trying to re-adopt the device with the attributes that
        // have been passed.
        let meta_dev = LinearDev::new(&format_flex_name(&pool_uuid, FlexRole::ThinMeta),
                                      dm,
                                      meta_segments)?;
        wipe_sectors(&meta_dev.devnode()?,
                     Sectors(0),
                     ThinPool::initial_metadata_size())?;

        let data_dev = LinearDev::new(&format_flex_name(&pool_uuid, FlexRole::ThinData),
                                      dm,
                                      data_segments)?;

        let mdv_name = format_flex_name(&pool_uuid, FlexRole::MetadataVolume);
        let mdv_dev = LinearDev::new(&mdv_name, dm, mdv_segments)?;
        let mdv = MetadataVol::initialize(&pool_uuid, mdv_dev)?;

        let name = format_thinpool_name(&pool_uuid, ThinPoolRole::Pool);
        let thinpool_dev = ThinPoolDev::new(name.as_ref(),
                                            dm,
                                            data_block_size,
                                            low_water_mark,
                                            meta_dev,
                                            data_dev)?;
        Ok(ThinPool {
               thin_pool: thinpool_dev,
               meta_spare: spare_segments,
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
    #[allow(too_many_arguments)]
    pub fn setup(pool_uuid: PoolUuid,
                 dm: &DM,
                 data_block_size: Sectors,
                 low_water_mark: DataBlocks,
                 spare_segments: Vec<Segment>,
                 meta_segments: Vec<Segment>,
                 data_segments: Vec<Segment>,
                 mdv_segments: Vec<Segment>)
                 -> EngineResult<ThinPool> {
        let meta_dev = LinearDev::new(&format_flex_name(&pool_uuid, FlexRole::ThinMeta),
                                      dm,
                                      meta_segments)?;

        let data_dev = LinearDev::new(&format_flex_name(&pool_uuid, FlexRole::ThinData),
                                      dm,
                                      data_segments)?;


        let name = format_thinpool_name(&pool_uuid, ThinPoolRole::Pool);

        let res = match ThinPoolDev::setup(name.as_ref(),
                                           dm,
                                           data_block_size,
                                           low_water_mark,
                                           meta_dev,
                                           data_dev) {
            Ok(dev) => Ok((dev, spare_segments)),
            Err(DmError::Dm(CheckFailed(meta_dev, data_dev), _)) => {
                attempt_thin_repair(pool_uuid, dm, meta_dev, spare_segments)
                    .and_then(|(new_meta_dev, new_spare_segments)| {
                        ThinPoolDev::setup(name.as_ref(),
                                           dm,
                                           data_block_size,
                                           low_water_mark,
                                           new_meta_dev,
                                           data_dev)
                                .map(|dev| (dev, new_spare_segments))
                                .map_err(|e| e.into())
                    })
            }
            Err(e) => Err(e.into()),
        };
        let (thinpool_dev, spare_segments) = res?;

        let mdv_dev = LinearDev::new(&format_flex_name(&pool_uuid, FlexRole::MetadataVolume),
                                     dm,
                                     mdv_segments)?;
        let mdv = MetadataVol::setup(&pool_uuid, mdv_dev)?;
        let filesystem_metadatas = mdv.filesystems()?;

        // TODO: not fail completely if one filesystem setup fails?
        let filesystems = {
            // Set up a filesystem from its metadata.
            let get_filesystem = |fssave: &FilesystemSave| -> EngineResult<StratFilesystem> {
                let device_name = format_thin_name(&pool_uuid, ThinRole::Filesystem(fssave.uuid));
                let thin_dev = ThinDev::setup(device_name.as_ref(),
                                              dm,
                                              &thinpool_dev,
                                              fssave.thin_id,
                                              fssave.size)?;
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
               thin_pool: thinpool_dev,
               meta_spare: spare_segments,
               id_gen: ThinDevIdPool::new_from_ids(&thin_ids),
               filesystems: fs_table,
               mdv: mdv,
           })
    }


    /// Initial size for a pool.
    pub fn initial_size() -> Sectors {
        // One extra meta for spare
        ThinPool::initial_metadata_size() * 2u64 + ThinPool::initial_data_size() +
        ThinPool::initial_mdv_size()
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

    /// The status of the thin pool as calculated by DM.
    pub fn check(&mut self, dm: &DM) -> EngineResult<ThinPoolStatus> {
        let thinpool = self.thin_pool.status(dm)?;
        self.mdv.check()?;

        let filesystems = self.filesystems
            .borrow_mut()
            .into_iter()
            .map(|fs| fs.check(dm))
            .collect::<EngineResult<Vec<_>>>()?;

        Ok(ThinPoolStatus {
               thinpool,
               filesystems,
           })
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

    /// Return the FlexDevsSave data structure for variable length metadata.
    /// May return an error if mapper can not locate the UUID corresponding
    /// to a device node.
    pub fn flexdevssave<F>(&self, mapper: F) -> EngineResult<FlexDevsSave>
        where F: Fn(&Segment) -> EngineResult<(Uuid, Sectors, Sectors)>
    {
        Ok(FlexDevsSave {
               meta_dev: self.mdv
                   .segments()
                   .iter()
                   .map(&mapper)
                   .collect::<EngineResult<Vec<_>>>()?,
               thin_meta_dev: self.thin_pool
                   .meta_dev()
                   .segments()
                   .iter()
                   .map(&mapper)
                   .collect::<EngineResult<Vec<_>>>()?,
               thin_data_dev: self.thin_pool
                   .data_dev()
                   .segments()
                   .iter()
                   .map(&mapper)
                   .collect::<EngineResult<Vec<_>>>()?,
               thin_meta_dev_spare: self.meta_spare
                   .iter()
                   .map(&mapper)
                   .collect::<EngineResult<Vec<_>>>()?,
           })
    }

    /// Get the devicemapper::ThinPoolDev for this pool. Used for testing.
    pub fn thinpooldev(&self) -> &ThinPoolDev {
        &self.thin_pool
    }

    /// Extend the thinpool with new data regions.
    pub fn extend_data(&mut self, dm: &DM, segs: Vec<Segment>) -> EngineResult<()> {
        Ok(self.thin_pool.extend_data(dm, segs)?)
    }

    /// The number of physical sectors in use, that is, unavailable for storage
    /// of additional user data, by this pool.
    // This includes all the sectors being held as spares for the meta device,
    // all the sectors allocated to the meta data device, and all the sectors
    // in use on the data device.
    pub fn total_physical_used(&self) -> EngineResult<Sectors> {
        let data_dev_used = match self.thin_pool.status(&DM::new()?)? {
            dm::ThinPoolStatus::Good(_, usage) => *usage.used_data * DATA_BLOCK_SIZE,
            _ => {
                let err_msg = "thin pool failed, could not obtain usage";
                return Err(EngineError::Engine(ErrorEnum::Invalid, err_msg.into()));
            }
        };

        let spare_total = self.meta_spare.iter().map(|s| s.length).sum();
        let meta_dev_total = self.thin_pool
            .meta_dev()
            .segments()
            .iter()
            .map(|s| s.length)
            .sum();

        let mdv_total = self.mdv.segments().iter().map(|s| s.length).sum();

        Ok(data_dev_used + spare_total + meta_dev_total + mdv_total)
    }

    pub fn get_filesystem_by_uuid(&self, uuid: &FilesystemUuid) -> Option<&StratFilesystem> {
        self.filesystems.get_by_uuid(uuid)
    }

    pub fn get_mut_filesystem_by_uuid(&mut self,
                                      uuid: &FilesystemUuid)
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
                             pool_uuid: &Uuid,
                             name: &str,
                             dm: &DM,
                             size: Option<Sectors>)
                             -> EngineResult<FilesystemUuid> {
        let fs_uuid = Uuid::new_v4();
        let device_name = format_thin_name(pool_uuid, ThinRole::Filesystem(fs_uuid));
        let thin_dev = ThinDev::new(device_name.as_ref(),
                                    dm,
                                    &self.thin_pool,
                                    self.id_gen.new_id()?,
                                    size.unwrap_or(DEFAULT_THIN_DEV_SIZE))?;

        let new_filesystem = StratFilesystem::initialize(fs_uuid, name, thin_dev)?;
        self.mdv.save_fs(&new_filesystem)?;
        self.filesystems.insert(new_filesystem);

        Ok(fs_uuid)
    }

    /// Destroy a filesystem within the thin pool.
    pub fn destroy_filesystem(&mut self, dm: &DM, uuid: &FilesystemUuid) -> EngineResult<()> {
        if let Some(fs) = self.filesystems.remove_by_uuid(uuid) {
            fs.destroy(dm, &self.thin_pool)?;
            self.mdv.rm_fs(uuid)?;
        }
        Ok(())
    }

    /// Rename a filesystem within the thin pool.
    pub fn rename_filesystem(&mut self,
                             uuid: &FilesystemUuid,
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
}

impl Recordable<ThinPoolDevSave> for ThinPool {
    fn record(&self) -> EngineResult<ThinPoolDevSave> {
        Ok(ThinPoolDevSave { data_block_size: self.thin_pool.data_block_size() })
    }
}

/// Attempt a thin repair operation on the meta device.
/// If the operation succeeds, teardown the old meta device,
/// and return the new meta device and the new spare segments.
fn attempt_thin_repair(pool_uuid: PoolUuid,
                       dm: &DM,
                       meta_dev: LinearDev,
                       mut spare_segments: Vec<Segment>)
                       -> EngineResult<(LinearDev, Vec<Segment>)> {
    let mut new_meta_dev = LinearDev::new(format_flex_name(&pool_uuid, FlexRole::ThinMetaSpare)
                                              .as_ref(),
                                          dm,
                                          spare_segments.drain(..).collect())?;


    if !Command::new("thin_repair")
            .arg("-i")
            .arg(&meta_dev.devnode()?)
            .arg("-o")
            .arg(&new_meta_dev.devnode()?)
            .status()?
            .success() {
        return Err(EngineError::Engine(ErrorEnum::Error,
                                       "thin_repair failed, pool unusable".into()));
    }

    let name = meta_dev.name().to_owned();
    let new_spare_segments = meta_dev
        .segments()
        .iter()
        .map(|x| {
                 Segment {
                     start: x.start,
                     length: x.length,
                     device: x.device,
                 }
             })
        .collect();
    meta_dev.teardown(dm)?;
    new_meta_dev.set_name(dm, name.as_ref())?;

    Ok((new_meta_dev, new_spare_segments))
}

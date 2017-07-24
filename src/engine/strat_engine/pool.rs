// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::{HashMap, HashSet};
use std::collections::hash_map::RandomState;
use std::iter::FromIterator;
use std::path::Path;
use std::path::PathBuf;
use std::vec::Vec;

use serde_json;
use uuid::Uuid;

use devicemapper::consts::SECTOR_SIZE;
use devicemapper::Device;
use devicemapper::DM;
use devicemapper::{DataBlocks, MetaBlocks, Sectors, Segment};
use devicemapper::LinearDev;
use devicemapper::{ThinDevId, ThinPoolStatus, ThinPoolWorkingStatus};

use super::super::consts::IEC::Mi;
use super::super::engine::{Filesystem, HasName, HasUuid, Pool};
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::structures::Table;
use super::super::types::{DevUuid, FilesystemUuid, PoolUuid, RenameAction, Redundancy};

use super::blockdevmgr::BlockDevMgr;
use super::device::wipe_sectors;
use super::dmdevice::{FlexRole, format_flex_name, format_thin_name, ThinRole};
use super::filesystem::{StratFilesystem, FilesystemStatus};
use super::mdv::MetadataVol;
use super::metadata::MIN_MDA_SECTORS;
use super::serde_structs::{FilesystemSave, FlexDevsSave, PoolSave, Recordable};
use super::setup::{get_blockdevs, get_metadata};
use super::thinpool::{META_LOWATER, ThinPool};

pub use super::thinpool::{DATA_BLOCK_SIZE, DATA_LOWATER};

const INITIAL_META_SIZE: MetaBlocks = MetaBlocks(4096);
pub const INITIAL_DATA_SIZE: DataBlocks = DataBlocks(768);
const INITIAL_MDV_SIZE: Sectors = Sectors(16 * Mi / SECTOR_SIZE as u64);

#[derive(Debug)]
pub struct StratPool {
    name: String,
    pool_uuid: PoolUuid,
    block_devs: BlockDevMgr,
    pub filesystems: Table<StratFilesystem>,
    redundancy: Redundancy,
    thin_pool: ThinPool,
    mdv: MetadataVol,
}

impl StratPool {
    /// Initialize a Stratis Pool.
    /// 1. Initialize the block devices specified by paths.
    /// 2. Set up thinpool device to back filesystems.
    pub fn initialize(name: &str,
                      dm: &DM,
                      paths: &[&Path],
                      redundancy: Redundancy,
                      force: bool)
                      -> EngineResult<(StratPool, Vec<PathBuf>)> {
        let pool_uuid = Uuid::new_v4();

        let mut block_mgr =
            try!(BlockDevMgr::initialize(&pool_uuid, paths, MIN_MDA_SECTORS, force));

        if block_mgr.avail_space() < StratPool::min_initial_size() {
            let avail_size = block_mgr.avail_space().bytes();

            // TODO: check the return value and update state machine on failure
            let _ = block_mgr.destroy_all();

            return Err(EngineError::Engine(ErrorEnum::Invalid,
                                           format!("Space on pool must be at least {} bytes, \
                                                   available space is only {} bytes",
                                                   StratPool::min_initial_size().bytes(),
                                                   avail_size)));


        }

        let meta_regions = block_mgr
            .alloc_space(INITIAL_META_SIZE.sectors())
            .expect("blockmgr must not fail, already checked for space");

        let meta_spare_regions = block_mgr
            .alloc_space(INITIAL_META_SIZE.sectors())
            .expect("blockmgr must not fail, already checked for space");

        let data_regions = block_mgr
            .alloc_space(*INITIAL_DATA_SIZE * DATA_BLOCK_SIZE)
            .expect("blockmgr must not fail, already checked for space");

        // When constructing a thin-pool, Stratis reserves the first N
        // sectors on a block device by creating a linear device with a
        // starting offset. DM writes the super block in the first block.
        // DM requires this first block to be zeros when the meta data for
        // the thin-pool is initially created. If we don't zero the
        // superblock DM issue error messages because it triggers code paths
        // that are trying to re-adopt the device with the attributes that
        // have been passed.
        let meta_dev = try!(LinearDev::new(&format_flex_name(&pool_uuid, FlexRole::ThinMeta),
                                           dm,
                                           meta_regions));
        try!(wipe_sectors(&try!(meta_dev.devnode()),
                          Sectors(0),
                          INITIAL_META_SIZE.sectors()));

        let data_dev = try!(LinearDev::new(&format_flex_name(&pool_uuid, FlexRole::ThinData),
                                           dm,
                                           data_regions));

        let thinpool = try!(ThinPool::new(pool_uuid,
                                          dm,
                                          DATA_BLOCK_SIZE,
                                          DATA_LOWATER,
                                          meta_spare_regions,
                                          meta_dev,
                                          data_dev));

        let mdv_regions = block_mgr
            .alloc_space(INITIAL_MDV_SIZE)
            .expect("blockmgr must not fail, already checked for space");

        let mdv_name = format_flex_name(&pool_uuid, FlexRole::MetadataVolume);
        let mdv_dev = try!(LinearDev::new(&mdv_name, dm, mdv_regions));
        let mdv = try!(MetadataVol::initialize(&pool_uuid, mdv_dev));

        let devnodes = block_mgr.devnodes();

        let mut pool = StratPool {
            name: name.to_owned(),
            pool_uuid: pool_uuid,
            block_devs: block_mgr,
            filesystems: Table::default(),
            redundancy: redundancy,
            thin_pool: thinpool,
            mdv: mdv,
        };

        try!(pool.write_metadata());

        Ok((pool, devnodes))
    }

    /// Setup a StratPool using its UUID and the list of devnodes it has.
    // TODO: Clean up after errors that occur after some action has been
    // taken on the environment.
    pub fn setup(uuid: PoolUuid, devnodes: &[PathBuf]) -> EngineResult<StratPool> {
        let metadata = try!(try!(get_metadata(uuid, devnodes))
                                .ok_or_else(|| EngineError::Engine(ErrorEnum::NotFound,
                                                           format!("no metadata for pool {}",
                                                                   uuid))));
        let blockdevs = try!(get_blockdevs(uuid, &metadata, devnodes));

        let uuid_map: HashMap<DevUuid, Device> = blockdevs
            .iter()
            .map(|bd| (*bd.uuid(), *bd.device()))
            .collect();

        /// Obtain a Segment from a Uuid, Sectors, Sectors triple.
        /// This can fail if there is no entry for the UUID in the map
        /// from UUIDs to device numbers.
        let lookup = |triple: &(Uuid, Sectors, Sectors)| -> EngineResult<Segment> {
            let device = try!(uuid_map
                                  .get(&triple.0)
                                  .ok_or_else(|| EngineError::Engine(ErrorEnum::NotFound,
                                                             format!("missing device for UUID {:?}",
                                                                     &triple.0))));
            Ok(Segment {
                   device: *device,
                   start: triple.1,
                   length: triple.2,
               })
        };

        let meta_segments: Vec<Segment> = try!(metadata
                                                   .flex_devs
                                                   .meta_dev
                                                   .iter()
                                                   .map(&lookup)
                                                   .collect());

        let thin_meta_segments: Vec<Segment> = try!(metadata
                                                        .flex_devs
                                                        .thin_meta_dev
                                                        .iter()
                                                        .map(&lookup)
                                                        .collect());

        let thin_data_segments: Vec<Segment> = try!(metadata
                                                        .flex_devs
                                                        .thin_data_dev
                                                        .iter()
                                                        .map(&lookup)
                                                        .collect());

        let thin_meta_spare_segments: Vec<Segment> = try!(metadata
                                                              .flex_devs
                                                              .thin_meta_dev_spare
                                                              .iter()
                                                              .map(&lookup)
                                                              .collect());

        let dm = try!(DM::new());

        // This is the cleanup zone.
        let meta_dev = try!(LinearDev::new(&format_flex_name(&uuid, FlexRole::ThinMeta),
                                           &dm,
                                           thin_meta_segments));

        let data_dev = try!(LinearDev::new(&format_flex_name(&uuid, FlexRole::ThinData),
                                           &dm,
                                           thin_data_segments));

        let mdv_dev = try!(LinearDev::new(&format_flex_name(&uuid, FlexRole::MetadataVolume),
                                          &dm,
                                          meta_segments));
        let mdv = try!(MetadataVol::setup(&uuid, mdv_dev));
        let filesystem_metadatas = try!(mdv.filesystems());
        let thin_ids: Vec<ThinDevId> = filesystem_metadatas.iter().map(|x| x.thin_id).collect();

        let thinpool = try!(ThinPool::setup(uuid,
                                            &dm,
                                            metadata.thinpool_dev.data_block_size,
                                            DATA_LOWATER,
                                            &thin_ids,
                                            thin_meta_spare_segments,
                                            meta_dev,
                                            data_dev));


        let filesystems: Vec<StratFilesystem> = {
            /// Set up a filesystem from its metadata.
            let get_filesystem = |fssave: &FilesystemSave| -> EngineResult<StratFilesystem> {
                let thin_dev = try!(thinpool.setup_thin_device(&dm,
                                    &format_thin_name(&uuid, ThinRole::Filesystem(fssave.uuid)),
                                    fssave.thin_id,
                                    fssave.size));
                Ok(try!(StratFilesystem::setup(fssave.uuid, &fssave.name, thin_dev)))
            };

            try!(filesystem_metadatas
                     .iter()
                     .map(get_filesystem)
                     .collect())
        };

        let mut table = Table::default();
        for fs in filesystems {
            let evicted = table.insert(fs);
            if !evicted.is_empty() {
                let err_msg = "filesystems with duplicate UUID or name specified in metadata";
                return Err(EngineError::Engine(ErrorEnum::Invalid, err_msg.into()));
            }
        }

        Ok(StratPool {
               name: metadata.name,
               pool_uuid: uuid,
               block_devs: BlockDevMgr::new(blockdevs),
               filesystems: table,
               redundancy: Redundancy::NONE,
               thin_pool: thinpool,
               mdv: mdv,
           })
    }

    /// Minimum initial size for a pool.
    pub fn min_initial_size() -> Sectors {
        // One extra meta for spare
        (INITIAL_META_SIZE.sectors() * 2u64) + *INITIAL_DATA_SIZE * DATA_BLOCK_SIZE +
        INITIAL_MDV_SIZE
    }

    /// Write current metadata to pool members.
    pub fn write_metadata(&mut self) -> EngineResult<()> {
        let data = try!(serde_json::to_string(&try!(self.record())));
        self.block_devs.save_state(data.as_bytes())
    }
    /// Return an extend size for the physical space backing a pool
    /// TODO: returning the current size will double the space provisoned to
    /// back the pool.  We should determine if this is a reasonable value.
    fn extend_size(&self, current_size: DataBlocks) -> DataBlocks {
        current_size
    }

    /// Expand the physical space allocated to a pool by the value from extend_size()
    /// Return the number of DataBlocks added
    fn extend_data(&mut self, dm: &DM, current_size: DataBlocks) -> EngineResult<DataBlocks> {
        let extend_size = self.extend_size(current_size);
        if let Some(new_data_regions) =
            self.block_devs
                .alloc_space(*extend_size * DATA_BLOCK_SIZE) {
            try!(self.thin_pool.extend_data(dm, new_data_regions));
        } else {
            let err_msg = format!("Insufficient space to accomodate request for {} data blocks",
                                  *extend_size);
            return Err(EngineError::Engine(ErrorEnum::Error, err_msg));
        }
        Ok(extend_size)
    }

    pub fn check(&mut self) -> () {
        #![allow(match_same_arms)]
        let dm = DM::new().expect("Could not get DM handle");

        if let Err(e) = self.mdv.check() {
            error!("MDV error: {}", e);
            return;
        }

        let result = match self.thin_pool.thin_pool_status(&dm) {
            Ok(r) => r,
            Err(_) => {
                error!("Could not get thinpool status");
                // TODO: Take pool offline?
                return;
            }
        };

        match result {
            ThinPoolStatus::Good(wstatus, usage) => {
                match wstatus {
                    ThinPoolWorkingStatus::Good => {}
                    ThinPoolWorkingStatus::ReadOnly => {
                        // TODO: why is pool r/o and how do we get it
                        // rw again?
                    }
                    ThinPoolWorkingStatus::OutOfSpace => {
                        // TODO: Add more space if possible, or
                        // prevent further usage
                        // Should never happen -- we should be extending first!
                    }
                    ThinPoolWorkingStatus::NeedsCheck => {
                        // TODO: Take pool offline?
                        // TODO: run thin_check
                    }
                }

                if usage.used_meta > usage.total_meta - META_LOWATER {
                    // TODO: Extend meta device
                }

                if usage.used_data > usage.total_data - DATA_LOWATER {
                    // Request expansion of physical space allocated to the pool
                    match self.extend_data(&dm, usage.total_data) {
                        #![allow(single_match)]
                        Ok(_) => {}
                        Err(_) => {} // TODO: Take pool offline?
                    }
                }
            }
            ThinPoolStatus::Fail => {
                // TODO: Take pool offline?
                // TODO: Run thin_check
            }
        }

        for fs in &mut self.filesystems {
            match fs.check(&dm) {
                Ok(f_status) => {
                    if let FilesystemStatus::Failed = f_status {
                        // TODO: recover fs? (how?)
                    }
                }
                Err(_e) => error!("fs.check() failed"),
            }
        }
    }

    /// Teardown a pool.
    /// Take down the device mapper devices belonging to the pool.
    /// Precondition: All filesystems belonging to this pool must be
    /// unmounted.
    pub fn teardown(self) -> EngineResult<()> {
        teardown_pool!(self);
        Ok(())
    }
}

impl Pool for StratPool {
    fn create_filesystems<'a, 'b>(&'a mut self,
                                  specs: &[&'b str])
                                  -> EngineResult<Vec<(&'b str, FilesystemUuid)>> {
        let names: HashSet<_, RandomState> = HashSet::from_iter(specs);
        for name in &names {
            if self.filesystems.contains_name(name) {
                return Err(EngineError::Engine(ErrorEnum::AlreadyExists, name.to_string()));
            }
        }

        // TODO: Roll back on filesystem initialization failure.
        let dm = try!(DM::new());
        let mut result = Vec::new();
        for name in &names {
            let uuid = Uuid::new_v4();
            let thin_dev = try!(self.thin_pool
                         .make_thin_device(&dm,
                                           &format_thin_name(&self.pool_uuid,
                                                             ThinRole::Filesystem(uuid)),
                                           None));
            let new_filesystem = try!(StratFilesystem::initialize(uuid, name, thin_dev));
            try!(self.mdv.save_fs(&new_filesystem));
            self.filesystems.insert(new_filesystem);
            result.push((**name, uuid));
        }

        Ok(result)
    }

    fn add_blockdevs(&mut self, paths: &[&Path], force: bool) -> EngineResult<Vec<PathBuf>> {
        let bdev_paths = try!(self.block_devs.add(&self.pool_uuid, paths, force));
        try!(self.write_metadata());
        Ok(bdev_paths)
    }

    /// Precondition: All filesystems belonging to this pool must be
    /// unmounted.
    fn destroy(self) -> EngineResult<()> {
        teardown_pool!(self);
        try!(self.block_devs.destroy_all());
        Ok(())
    }

    fn destroy_filesystems<'a, 'b>(&'a mut self,
                                   fs_uuids: &[&'b FilesystemUuid])
                                   -> EngineResult<Vec<&'b FilesystemUuid>> {
        for fsid in fs_uuids {
            try!(self.mdv.rm_fs(fsid));
        }
        destroy_filesystems!{self; fs_uuids}
    }

    fn rename_filesystem(&mut self,
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

    fn rename(&mut self, name: &str) {
        self.name = name.to_owned();
    }

    fn get_filesystem(&mut self, uuid: &FilesystemUuid) -> Option<&mut Filesystem> {
        get_filesystem!(self; uuid)
    }

    fn total_physical_size(&self) -> Sectors {
        self.block_devs.current_capacity()
    }

    fn total_physical_used(&self) -> EngineResult<Sectors> {
        self.thin_pool
            .total_physical_used()
            .and_then(|v| {
                          Ok(v + self.block_devs.metadata_size() +
                             self.mdv.segments().iter().map(|s| s.length).sum())
                      })
    }
}

impl HasUuid for StratPool {
    fn uuid(&self) -> &PoolUuid {
        &self.pool_uuid
    }
}

impl HasName for StratPool {
    fn name(&self) -> &str {
        &self.name
    }
}

impl Recordable<PoolSave> for StratPool {
    fn record(&self) -> EngineResult<PoolSave> {

        let mapper = |seg: &Segment| -> EngineResult<(Uuid, Sectors, Sectors)> {
            let bd = try!(self.block_devs
                     .get_by_device(seg.device)
                     .ok_or_else(|| EngineError::Engine(ErrorEnum::NotFound,
                                                format!("no block device found for device {:?}",
                                                        seg.device))));
            Ok((*bd.uuid(), seg.start, seg.length))
        };

        let meta_dev = try!(self.mdv.segments().iter().map(&mapper).collect());

        let thin_meta_dev = try!(self.thin_pool
                                     .thin_pool_meta_segments()
                                     .iter()
                                     .map(&mapper)
                                     .collect());

        let thin_data_dev = try!(self.thin_pool
                                     .thin_pool_data_segments()
                                     .iter()
                                     .map(&mapper)
                                     .collect());

        let thin_meta_dev_spare = try!(self.thin_pool
                                           .spare_segments()
                                           .iter()
                                           .map(&mapper)
                                           .collect());

        Ok(PoolSave {
               name: self.name.clone(),
               block_devs: try!(self.block_devs.record()),
               flex_devs: FlexDevsSave {
                   meta_dev: meta_dev,
                   thin_meta_dev: thin_meta_dev,
                   thin_data_dev: thin_data_dev,
                   thin_meta_dev_spare: thin_meta_dev_spare,
               },
               thinpool_dev: self.thin_pool
                   .record()
                   .expect("this function never fails"),
           })
    }
}

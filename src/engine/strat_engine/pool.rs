// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashSet;
use std::collections::hash_map::RandomState;
use std::iter::FromIterator;
use std::path::Path;
use std::path::PathBuf;
use std::vec::Vec;

use devicemapper::consts::SECTOR_SIZE;
use devicemapper::DM;
use devicemapper::{DataBlocks, Sectors, Segment};
use devicemapper::LinearDev;
use devicemapper::{ThinPoolDev, ThinPoolStatus, ThinPoolWorkingStatus};
use time::now;
use uuid::Uuid;
use serde_json;

use super::super::consts::IEC::Mi;
use super::super::engine::{Filesystem, HasName, HasUuid, Pool};
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::structures::Table;
use super::super::types::{FilesystemUuid, PoolUuid, RenameAction, Redundancy};

use super::blockdevmgr::BlockDevMgr;
use super::device::wipe_sectors;
use super::dmdevice::{FlexRole, ThinPoolRole, format_flex_name, format_thinpool_name};
use super::filesystem::{StratFilesystem, FilesystemStatus};
use super::mdv::MetadataVol;
use super::metadata::MIN_MDA_SECTORS;
use super::serde_structs::{FlexDevsSave, PoolSave, Recordable, ThinPoolDevSave};

const DATA_BLOCK_SIZE: Sectors = Sectors(2048);
const META_LOWATER: u64 = 512;
const DATA_LOWATER: DataBlocks = DataBlocks(512);

const INITIAL_META_SIZE: Sectors = Sectors(16 * Mi / SECTOR_SIZE as u64);
const INITIAL_DATA_SIZE: Sectors = Sectors(512 * Mi / SECTOR_SIZE as u64);
const INITIAL_MDV_SIZE: Sectors = Sectors(16 * Mi / SECTOR_SIZE as u64);

#[derive(Debug)]
pub struct StratPool {
    name: String,
    pool_uuid: PoolUuid,
    pub block_devs: BlockDevMgr,
    pub filesystems: Table<StratFilesystem>,
    redundancy: Redundancy,
    thin_pool: ThinPoolDev,
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
                      -> EngineResult<StratPool> {
        let pool_uuid = Uuid::new_v4();

        let mut block_mgr =
            try!(BlockDevMgr::initialize(&pool_uuid, paths, MIN_MDA_SECTORS, force));

        if block_mgr.avail_space() < StratPool::min_initial_size() {
            let avail_size = block_mgr.avail_space().bytes();
            try!(block_mgr.destroy_all());
            return Err(EngineError::Engine(ErrorEnum::Invalid,
                                           format!("Space on pool must be at least {} bytes, \
                                                   available space is only {} bytes",
                                                   StratPool::min_initial_size().bytes(),
                                                   avail_size)));


        }

        let meta_regions = block_mgr
            .alloc_space(INITIAL_META_SIZE)
            .expect("blockmgr must not fail, already checked for space");

        let data_regions = block_mgr
            .alloc_space(INITIAL_DATA_SIZE)
            .expect("blockmgr must not fail, already checked for space");

        let thinpool_dev =
            try!(StratPool::setup_thinpooldev(dm, &pool_uuid, meta_regions, data_regions));

        let mdv_regions = block_mgr
            .alloc_space(INITIAL_MDV_SIZE)
            .expect("blockmgr must not fail, already checked for space");
        let device_name = format_flex_name(&pool_uuid, FlexRole::MetadataVolume);
        let mdv_dev = try!(LinearDev::new(&device_name, dm, mdv_regions));
        let mdv = try!(MetadataVol::initialize(&pool_uuid, mdv_dev));

        let mut pool = StratPool {
            name: name.to_owned(),
            pool_uuid: pool_uuid,
            block_devs: block_mgr,
            filesystems: Table::new(),
            redundancy: redundancy,
            thin_pool: thinpool_dev,
            mdv: mdv,
        };

        try!(pool.write_metadata());

        Ok(pool)
    }

    /// Setup a thin pool device for this pool from the designated segments.
    // TODO: complicate this method by optionally checking whether the device
    // already exists.
    pub fn setup_thinpooldev(dm: &DM,
                             pool_uuid: &PoolUuid,
                             meta_segs: Vec<Segment>,
                             data_segs: Vec<Segment>)
                             -> EngineResult<ThinPoolDev> {
        let device_name = format_flex_name(pool_uuid, FlexRole::ThinMeta);
        let meta_dev = try!(LinearDev::new(&device_name, dm, meta_segs));

        // When constructing a thin-pool, Stratis reserves the first N
        // sectors on a block device by creating a linear device with a
        // starting offset. DM writes the super block in the first block.
        // DM requires this first block to be zeros when the meta data for
        // the thin-pool is initially created. If we don't zero the
        // superblock DM issue error messages because it triggers code paths
        // that are trying to re-adopt the device with the attributes that
        // have been passed.
        try!(wipe_sectors(&try!(meta_dev.devnode()), Sectors(0), INITIAL_META_SIZE));

        let device_name = format_flex_name(pool_uuid, FlexRole::ThinData);
        let data_dev = try!(LinearDev::new(&device_name, dm, data_segs));

        let device_name = format_thinpool_name(&pool_uuid, ThinPoolRole::Pool);
        // TODO Fix hard coded data blocksize and low water mark.
        Ok(try!(ThinPoolDev::new(&device_name,
                                 dm,
                                 try!(data_dev.size()),
                                 DATA_BLOCK_SIZE,
                                 DataBlocks(256000),
                                 meta_dev,
                                 data_dev)))
    }

    /// Minimum initial size for a pool.
    pub fn min_initial_size() -> Sectors {
        INITIAL_META_SIZE + INITIAL_DATA_SIZE + INITIAL_MDV_SIZE
    }

    // TODO: Check current time against global last updated, and use
    // alternate time value if earlier, as described in SWDD
    fn write_metadata(&mut self) -> EngineResult<()> {
        let data = try!(serde_json::to_string(&try!(self.record())));
        self.block_devs
            .save_state(&now().to_timespec(), data.as_bytes())
    }

    pub fn check(&mut self) -> () {
        let dm = DM::new().expect("Could not get DM handle");

        if let Err(e) = self.mdv.check() {
            error!("MDV error: {}", e);
            return;
        }

        let result = match self.thin_pool.status(&dm) {
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
                    // TODO: Extend data device
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
    /// This method and destroy() must keep their DM teardown operations
    /// in sync.
    pub fn teardown(self) -> EngineResult<()> {
        // TODO: any necessary clean up of filesystems
        if !self.filesystems.is_empty() {
            return Err(EngineError::Engine(ErrorEnum::Busy,
                                           format!("May be unsynced files on device.")));
        }
        let dm = try!(DM::new());
        try!(self.thin_pool.teardown(&dm));
        try!(self.mdv.teardown(&dm));
        Ok(())
    }
}

impl Pool for StratPool {
    fn create_filesystems<'a, 'b>(&'a mut self,
                                  specs: &[&'b str])
                                  -> EngineResult<Vec<(&'b str, FilesystemUuid)>> {
        let names: HashSet<_, RandomState> = HashSet::from_iter(specs);
        for name in names.iter() {
            if self.filesystems.contains_name(name) {
                return Err(EngineError::Engine(ErrorEnum::AlreadyExists, name.to_string()));
            }
        }

        // TODO: Roll back on filesystem initialization failure.
        let dm = try!(DM::new());
        let mut result = Vec::new();
        for name in names.iter() {
            let uuid = Uuid::new_v4();
            let new_filesystem = try!(StratFilesystem::initialize(&self.pool_uuid,
                                                                  uuid,
                                                                  name,
                                                                  &dm,
                                                                  &mut self.thin_pool));
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

    fn destroy(self) -> EngineResult<()> {
        // Ensure that DM teardown operations in this method are in sync
        // with operations in teardown().
        let dm = try!(DM::new());
        try!(self.thin_pool.teardown(&dm));
        try!(self.mdv.teardown(&dm));
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
        rename_filesystem!{self; uuid; new_name}
    }

    fn rename(&mut self, name: &str) {
        self.name = name.to_owned();
    }

    fn get_filesystem(&mut self, uuid: &FilesystemUuid) -> Option<&mut Filesystem> {
        get_filesystem!(self; uuid)
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
                     .ok_or(EngineError::Engine(ErrorEnum::NotFound,
                                                format!("no block device found for device {:?}",
                                                        seg.device))));
            Ok((*bd.uuid(), seg.start, seg.length))
        };

        let meta_dev = try!(self.mdv.segments().iter().map(&mapper).collect());

        let thin_meta_dev = try!(self.thin_pool
                                     .meta_dev()
                                     .segments()
                                     .iter()
                                     .map(&mapper)
                                     .collect());

        let thin_data_dev = try!(self.thin_pool
                                     .data_dev()
                                     .segments()
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
               },
               thinpool_dev: ThinPoolDevSave { data_block_size: self.thin_pool.data_block_size() },
           })
    }
}

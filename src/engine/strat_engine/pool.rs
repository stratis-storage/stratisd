// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;
use std::iter::FromIterator;
use std::path::Path;
use std::path::PathBuf;
use std::vec::Vec;

use serde_json;
use uuid::Uuid;

use devicemapper::{Device, DM, Sectors, ThinPoolDev};

use super::super::engine::{Filesystem, BlockDev, HasName, HasUuid, Pool};
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::types::{FilesystemUuid, PoolUuid, RenameAction, Redundancy};

use super::blockdevmgr::BlockDevMgr;
use super::filesystem::StratFilesystem;
use super::metadata::MIN_MDA_SECTORS;
use super::serde_structs::{PoolSave, Recordable};
use super::setup::{get_blockdevs, get_metadata};
use super::thinpool::ThinPool;

pub use super::thinpool::{DATA_BLOCK_SIZE, DATA_LOWATER, INITIAL_DATA_SIZE};

#[derive(Debug)]
pub struct StratPool {
    name: String,
    pool_uuid: PoolUuid,
    block_devs: BlockDevMgr,
    redundancy: Redundancy,
    thin_pool: ThinPool,
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

        let mut block_mgr = BlockDevMgr::initialize(&pool_uuid, paths, MIN_MDA_SECTORS, force)?;

        let thinpool = ThinPool::new(pool_uuid, dm, DATA_BLOCK_SIZE, DATA_LOWATER, &mut block_mgr);
        let thinpool = match thinpool {
            Ok(thinpool) => thinpool,
            Err(err) => {
                let _ = block_mgr.destroy_all();
                return Err(err);
            }
        };

        let devnodes = block_mgr.devnodes();

        let mut pool = StratPool {
            name: name.to_owned(),
            pool_uuid: pool_uuid,
            block_devs: block_mgr,
            redundancy: redundancy,
            thin_pool: thinpool,
        };

        pool.write_metadata()?;

        Ok((pool, devnodes))
    }

    /// Setup a StratPool using its UUID and the list of devnodes it has.
    pub fn setup(uuid: PoolUuid, devnodes: &HashMap<Device, PathBuf>) -> EngineResult<StratPool> {
        let metadata = get_metadata(uuid, devnodes)?
            .ok_or_else(|| {
                            EngineError::Engine(ErrorEnum::NotFound,
                                                format!("no metadata for pool {}", uuid))
                        })?;
        let bd_mgr = BlockDevMgr::new(get_blockdevs(uuid, &metadata, devnodes)?);
        let thinpool = ThinPool::setup(uuid,
                                       &DM::new()?,
                                       metadata.thinpool_dev.data_block_size,
                                       DATA_LOWATER,
                                       &metadata.flex_devs,
                                       &bd_mgr)?;

        Ok(StratPool {
               name: metadata.name,
               pool_uuid: uuid,
               block_devs: bd_mgr,
               redundancy: Redundancy::NONE,
               thin_pool: thinpool,
           })
    }

    /// Write current metadata to pool members.
    pub fn write_metadata(&mut self) -> EngineResult<()> {
        let data = serde_json::to_string(&self.record())?;
        self.block_devs.save_state(data.as_bytes())
    }

    pub fn check(&mut self) -> () {
        self.thin_pool
            // FIXME: It's wrong to invite a crash with an unwrap() here.
            .check(&DM::new().unwrap(), &mut self.block_devs)
            .unwrap_or(error!("Thin pool check did not succeed"))
    }

    /// Teardown a pool.
    pub fn teardown(self) -> EngineResult<()> {
        self.thin_pool.teardown(&DM::new()?)
    }

    /// Look up a filesystem in the pool.
    pub fn get_mut_strat_filesystem(&mut self,
                                    uuid: &FilesystemUuid)
                                    -> Option<&mut StratFilesystem> {
        self.thin_pool.get_mut_filesystem_by_uuid(uuid)
    }

    /// Get the devicemapper::ThinPoolDev for this pool. Used for testing.
    pub fn thinpooldev(&self) -> &ThinPoolDev {
        self.thin_pool.thinpooldev()
    }

    pub fn has_filesystems(&self) -> bool {
        self.thin_pool.has_filesystems()
    }
}

impl Pool for StratPool {
    fn create_filesystems<'a, 'b>(&'a mut self,
                                  specs: &[(&'b str, Option<Sectors>)])
                                  -> EngineResult<Vec<(&'b str, FilesystemUuid)>> {
        let names: HashMap<_, _> = HashMap::from_iter(specs.iter().map(|&tup| (tup.0, tup.1)));
        for name in names.keys() {
            if self.thin_pool
                   .get_mut_filesystem_by_name(*name)
                   .is_some() {
                return Err(EngineError::Engine(ErrorEnum::AlreadyExists, name.to_string()));
            }
        }

        // TODO: Roll back on filesystem initialization failure.
        let dm = DM::new()?;
        let mut result = Vec::new();
        for (name, size) in names {
            let fs_uuid = self.thin_pool
                .create_filesystem(&self.pool_uuid, name, &dm, size)?;
            result.push((name, fs_uuid));
        }

        Ok(result)
    }

    fn add_blockdevs(&mut self, paths: &[&Path], force: bool) -> EngineResult<Vec<PathBuf>> {
        let bdev_paths = self.block_devs.add(&self.pool_uuid, paths, force)?;
        self.write_metadata()?;
        Ok(bdev_paths)
    }

    fn destroy(self) -> EngineResult<()> {
        self.thin_pool.teardown(&DM::new()?)?;
        self.block_devs.destroy_all()?;
        Ok(())
    }

    fn destroy_filesystems<'a, 'b>(&'a mut self,
                                   fs_uuids: &[&'b FilesystemUuid])
                                   -> EngineResult<Vec<&'b FilesystemUuid>> {
        let dm = DM::new()?;

        let mut removed = Vec::new();
        for uuid in fs_uuids {
            self.thin_pool.destroy_filesystem(&dm, uuid)?;
            removed.push(*uuid);
        }

        Ok(removed)
    }

    fn rename_filesystem(&mut self,
                         uuid: &FilesystemUuid,
                         new_name: &str)
                         -> EngineResult<RenameAction> {
        self.thin_pool.rename_filesystem(uuid, new_name)
    }

    fn rename(&mut self, name: &str) {
        self.name = name.to_owned();
    }

    fn get_filesystem(&self, uuid: &FilesystemUuid) -> Option<&Filesystem> {
        self.thin_pool
            .get_filesystem_by_uuid(uuid)
            .map(|fs| fs as &Filesystem)
    }

    fn get_mut_filesystem(&mut self, uuid: &FilesystemUuid) -> Option<&mut Filesystem> {
        self.thin_pool
            .get_mut_filesystem_by_uuid(uuid)
            .map(|fs| fs as &mut Filesystem)
    }

    fn total_physical_size(&self) -> Sectors {
        self.block_devs.current_capacity()
    }

    fn total_physical_used(&self) -> EngineResult<Sectors> {
        self.thin_pool
            .total_physical_used()
            .and_then(|v| Ok(v + self.block_devs.metadata_size()))
    }

    fn filesystems(&self) -> Vec<&Filesystem> {
        self.thin_pool.filesystems()
    }

    fn blockdevs(&self) -> Vec<&BlockDev> {
        self.block_devs.blockdevs()
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
    fn record(&self) -> PoolSave {
        PoolSave {
            name: self.name.clone(),
            block_devs: self.block_devs.record(),
            flex_devs: self.thin_pool.record(),
            thinpool_dev: self.thin_pool.record(),
        }
    }
}

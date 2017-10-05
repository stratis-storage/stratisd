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

use devicemapper::{Device, DM, DmName, DmNameBuf, Sectors, ThinPoolDev};

use super::super::engine::{Filesystem, BlockDev, HasName, HasUuid, Pool};
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::types::{DevUuid, FilesystemUuid, PoolUuid, RenameAction, Redundancy};

use super::blockdevmgr::BlockDevMgr;
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
                      -> EngineResult<StratPool> {
        let pool_uuid = Uuid::new_v4();

        let mut block_mgr = BlockDevMgr::initialize(pool_uuid, paths, MIN_MDA_SECTORS, force)?;

        let thinpool = ThinPool::new(pool_uuid, dm, DATA_BLOCK_SIZE, DATA_LOWATER, &mut block_mgr);
        let thinpool = match thinpool {
            Ok(thinpool) => thinpool,
            Err(err) => {
                let _ = block_mgr.destroy_all();
                return Err(err);
            }
        };

        let mut pool = StratPool {
            name: name.to_owned(),
            pool_uuid: pool_uuid,
            block_devs: block_mgr,
            redundancy: redundancy,
            thin_pool: thinpool,
        };

        pool.write_metadata()?;

        Ok(pool)
    }

    /// Setup a StratPool using its UUID and the list of devnodes it has.
    pub fn setup(uuid: PoolUuid, devnodes: &HashMap<Device, PathBuf>) -> EngineResult<StratPool> {
        let metadata = get_metadata(uuid, devnodes)?
            .ok_or_else(|| {
                            EngineError::Engine(ErrorEnum::NotFound,
                                                format!("no metadata for pool {}", uuid))
                        })?;
        let bd_mgr = BlockDevMgr::new(uuid, get_blockdevs(uuid, &metadata, devnodes)?);
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

    pub fn check(&mut self) -> EngineResult<()> {
        // FIXME: The context should not be created here as this is not
        // a public method. Ideally the context should be created in the
        // invoking method, Engine::check(). However, since we hope that
        // method will go away entirely, we just fix half of the problem
        // with this method, and leave the rest alone.
        self.thin_pool.check(&DM::new()?, &mut self.block_devs)
    }

    /// Teardown a pool.
    pub fn teardown(self) -> EngineResult<()> {
        self.thin_pool.teardown(&DM::new()?)
    }

    pub fn has_filesystems(&self) -> bool {
        self.thin_pool.has_filesystems()
    }

    /// The names of DM devices belonging to this pool that may generate events
    pub fn get_eventing_dev_names(&self) -> Vec<DmNameBuf> {
        self.thin_pool.get_eventing_dev_names()
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
            let fs_uuid = self.thin_pool.create_filesystem(name, &dm, size)?;
            result.push((name, fs_uuid));
        }

        Ok(result)
    }

    fn add_blockdevs(&mut self, paths: &[&Path], force: bool) -> EngineResult<Vec<DevUuid>> {
        let bdev_info = self.block_devs.add(paths, force)?;
        self.write_metadata()?;
        Ok(bdev_info)
    }

    fn destroy(self) -> EngineResult<()> {
        self.thin_pool.teardown(&DM::new()?)?;
        self.block_devs.destroy_all()?;
        Ok(())
    }

    fn destroy_filesystems<'a>(&'a mut self,
                               fs_uuids: &[FilesystemUuid])
                               -> EngineResult<Vec<FilesystemUuid>> {
        let dm = DM::new()?;

        let mut removed = Vec::new();
        for &uuid in fs_uuids {
            self.thin_pool.destroy_filesystem(&dm, uuid)?;
            removed.push(uuid);
        }

        Ok(removed)
    }

    fn rename_filesystem(&mut self,
                         uuid: FilesystemUuid,
                         new_name: &str)
                         -> EngineResult<RenameAction> {
        self.thin_pool.rename_filesystem(uuid, new_name)
    }

    fn rename(&mut self, name: &str) {
        self.name = name.to_owned();
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

    fn snapshot_filesystem(&mut self,
                           origin_uuid: FilesystemUuid,
                           snapshot_name: &str)
                           -> EngineResult<FilesystemUuid> {
        self.thin_pool
            .snapshot_filesystem(&DM::new()?, origin_uuid, snapshot_name)
    }

    fn get_filesystem(&self, uuid: FilesystemUuid) -> Option<&Filesystem> {
        self.thin_pool
            .get_filesystem_by_uuid(uuid)
            .map(|fs| fs as &Filesystem)
    }

    fn get_mut_filesystem(&mut self, uuid: FilesystemUuid) -> Option<&mut Filesystem> {
        self.thin_pool
            .get_mut_filesystem_by_uuid(uuid)
            .map(|fs| fs as &mut Filesystem)
    }

    fn blockdevs(&self) -> Vec<&BlockDev> {
        self.block_devs.blockdevs()
    }

    fn get_blockdev(&self, uuid: DevUuid) -> Option<&BlockDev> {
        self.block_devs.get_blockdev_by_uuid(uuid)
    }

    fn get_mut_blockdev(&mut self, uuid: DevUuid) -> Option<&mut BlockDev> {
        self.block_devs.get_mut_blockdev_by_uuid(uuid)
    }

    fn save_state(&mut self) -> EngineResult<()> {
        self.write_metadata()
    }
}

impl HasUuid for StratPool {
    fn uuid(&self) -> PoolUuid {
        self.pool_uuid
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

#[cfg(test)]
mod tests {
    use super::super::super::types::Redundancy;

    use super::super::setup::find_all;
    use super::super::tests::{loopbacked, real};

    use super::*;

    /// Verify that metadata can be read from pools.
    /// 1. Split paths into two separate sets.
    /// 2. Create pools from the two sets.
    /// 3. Use find_all() to get the devices in the pool.
    /// 4. Use get_metadata to find metadata for each pool and verify
    /// correctness.
    /// 5. Teardown the engine and repeat.
    fn test_basic_metadata(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let (paths1, paths2) = paths.split_at(paths.len() / 2);
        let dm = DM::new().unwrap();

        let name1 = "name1";
        let pool1 = StratPool::initialize(&name1, &dm, paths1, Redundancy::NONE, false).unwrap();
        let uuid1 = pool1.uuid();
        let metadata1 = pool1.record();

        let name2 = "name2";
        let pool2 = StratPool::initialize(&name2, &dm, paths2, Redundancy::NONE, false).unwrap();
        let uuid2 = pool2.uuid();
        let metadata2 = pool2.record();

        let pools = find_all().unwrap();
        assert_eq!(pools.len(), 2);
        let devnodes1 = pools.get(&uuid1).unwrap();
        let devnodes2 = pools.get(&uuid2).unwrap();
        let pool_save1 = get_metadata(uuid1, devnodes1).unwrap().unwrap();
        let pool_save2 = get_metadata(uuid2, devnodes2).unwrap().unwrap();
        assert_eq!(pool_save1, metadata1);
        assert_eq!(pool_save2, metadata2);
        let blockdevs1 = get_blockdevs(uuid1, &pool_save1, devnodes1).unwrap();
        let blockdevs2 = get_blockdevs(uuid2, &pool_save2, devnodes2).unwrap();
        assert_eq!(blockdevs1.len(), pool_save1.block_devs.len());
        assert_eq!(blockdevs2.len(), pool_save2.block_devs.len());

        pool1.teardown().unwrap();
        pool2.teardown().unwrap();
        let pools = find_all().unwrap();
        assert_eq!(pools.len(), 2);
        let devnodes1 = pools.get(&uuid1).unwrap();
        let devnodes2 = pools.get(&uuid2).unwrap();
        let pool_save1 = get_metadata(uuid1, devnodes1).unwrap().unwrap();
        let pool_save2 = get_metadata(uuid2, devnodes2).unwrap().unwrap();
        assert_eq!(pool_save1, metadata1);
        assert_eq!(pool_save2, metadata2);
        let blockdevs1 = get_blockdevs(uuid1, &pool_save1, devnodes1).unwrap();
        let blockdevs2 = get_blockdevs(uuid2, &pool_save2, devnodes2).unwrap();
        assert_eq!(blockdevs1.len(), pool_save1.block_devs.len());
        assert_eq!(blockdevs2.len(), pool_save2.block_devs.len());
    }

    #[test]
    pub fn loop_test_basic_metadata() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(2, 3), test_basic_metadata);
    }

    #[test]
    pub fn real_test_basic_metadata() {
        real::test_with_spec(real::DeviceLimits::AtLeast(2), test_basic_metadata);
    }
    /// Verify that a pool with no devices does not have the minimum amount of
    /// space required.
    fn test_empty_pool(paths: &[&Path]) -> () {
        assert_eq!(paths.len(), 0);
        let dm = DM::new().unwrap();
        assert!(match StratPool::initialize("stratis_test_pool",
                                            &dm,
                                            paths,
                                            Redundancy::NONE,
                                            true)
                              .unwrap_err() {
                    EngineError::Engine(ErrorEnum::Invalid, _) => true,
                    _ => false,
                });
    }

    #[test]
    pub fn loop_test_empty_pool() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Exactly(0), test_empty_pool);
    }

    #[test]
    pub fn real_test_empty_pool() {
        real::test_with_spec(real::DeviceLimits::Exactly(0), test_empty_pool);
    }
}

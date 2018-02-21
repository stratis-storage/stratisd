// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::vec::Vec;

use serde_json;
use uuid::Uuid;

use devicemapper::{DM, Device, DmName, DmNameBuf, Sectors};

use super::super::engine::{BlockDev, Filesystem, Pool};
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::types::{BlockDevTier, DevUuid, FilesystemUuid, Name, PoolUuid, Redundancy,
                          RenameAction};

use super::backstore::{Backstore, MIN_MDA_SECTORS, get_metadata};
use super::serde_structs::{PoolSave, Recordable};
use super::thinpool::{ThinPool, ThinPoolSizeParams};

pub use super::thinpool::{DATA_BLOCK_SIZE, DATA_LOWATER, INITIAL_DATA_SIZE};

#[derive(Debug)]
pub struct StratPool {
    backstore: Backstore,
    redundancy: Redundancy,
    thin_pool: ThinPool,
}

impl StratPool {
    /// Initialize a Stratis Pool.
    /// 1. Initialize the block devices specified by paths.
    /// 2. Set up thinpool device to back filesystems.
    pub fn initialize(dm: &DM,
                      name: &str,
                      paths: &[&Path],
                      redundancy: Redundancy,
                      force: bool)
                      -> EngineResult<(PoolUuid, StratPool)> {
        let pool_uuid = Uuid::new_v4();

        let mut backstore = Backstore::initialize(dm, pool_uuid, paths, MIN_MDA_SECTORS, force)?;

        let thinpool = ThinPool::new(pool_uuid,
                                     dm,
                                     &ThinPoolSizeParams::default(),
                                     DATA_BLOCK_SIZE,
                                     DATA_LOWATER,
                                     &mut backstore);
        let thinpool = match thinpool {
            Ok(thinpool) => thinpool,
            Err(err) => {
                let _ = backstore.destroy(dm);
                return Err(err);
            }
        };

        let mut pool = StratPool {
            backstore,
            redundancy,
            thin_pool: thinpool,
        };

        pool.write_metadata(&Name::new(name.to_owned()))?;

        Ok((pool_uuid, pool))
    }

    /// Setup a StratPool using its UUID and the list of devnodes it has.
    pub fn setup(dm: &DM,
                 uuid: PoolUuid,
                 devnodes: &HashMap<Device, PathBuf>)
                 -> EngineResult<(Name, StratPool)> {
        let metadata = get_metadata(uuid, devnodes)?
            .ok_or_else(|| {
                            EngineError::Engine(ErrorEnum::NotFound,
                                                format!("no metadata for pool {}", uuid))
                        })?;

        // If the amount allocated from the cache tier is not the same as that
        // used by the thinpool, consider the situation an error.
        let flex_devs = &metadata.flex_devs;
        let total_allocated = flex_devs
            .meta_dev
            .iter()
            .chain(flex_devs.thin_meta_dev.iter())
            .chain(flex_devs.thin_data_dev.iter())
            .chain(flex_devs.thin_meta_dev_spare.iter())
            .map(|x| x.1)
            .sum::<Sectors>();
        if total_allocated != metadata.backstore.next {
            let err_msg = format!("{} used in thinpool, but {} given up by cache",
                                  total_allocated,
                                  metadata.backstore.next);
            return Err(EngineError::Engine(ErrorEnum::Invalid, err_msg));
        }

        let backstore = Backstore::setup(dm, uuid, &metadata.backstore, devnodes, None)?;
        let thinpool = ThinPool::setup(dm,
                                       uuid,
                                       metadata.thinpool_dev.data_block_size,
                                       DATA_LOWATER,
                                       &metadata.flex_devs,
                                       &backstore)?;

        Ok((Name::new(metadata.name),
            StratPool {
                backstore,
                redundancy: Redundancy::NONE,
                thin_pool: thinpool,
            }))
    }

    /// Write current metadata to pool members.
    pub fn write_metadata(&mut self, name: &str) -> EngineResult<()> {
        let data = serde_json::to_string(&self.record(name))?;
        self.backstore.datadev_save_state(data.as_bytes())
    }

    pub fn check(&mut self, name: &Name) -> EngineResult<()> {
        // FIXME: The context should not be created here as this is not
        // a public method. Ideally the context should be created in the
        // invoking method, Engine::check(). However, since we hope that
        // method will go away entirely, we just fix half of the problem
        // with this method, and leave the rest alone.
        if self.thin_pool.check(&DM::new()?, &mut self.backstore)? {
            self.write_metadata(name)?;
        }
        Ok(())
    }

    /// Teardown a pool.
    pub fn teardown(self, dm: &DM) -> EngineResult<()> {
        self.thin_pool.teardown(dm)?;
        self.backstore.teardown(dm)
    }

    pub fn has_filesystems(&self) -> bool {
        self.thin_pool.has_filesystems()
    }

    /// The names of DM devices belonging to this pool that may generate events
    pub fn get_eventing_dev_names(&self) -> Vec<DmNameBuf> {
        self.thin_pool.get_eventing_dev_names()
    }

    /// Called when a DM device in this pool has generated an event.
    // TODO: Just check the device that evented. Currently checks
    // everything.
    pub fn event_on(&mut self, pool_name: &Name, dm_name: &DmName) -> EngineResult<()> {
        assert!(self.thin_pool
                    .get_eventing_dev_names()
                    .iter()
                    .any(|x| dm_name == &**x));
        if self.thin_pool.check(&DM::new()?, &mut self.backstore)? {
            self.write_metadata(pool_name)?;
        }
        Ok(())
    }

    pub fn record(&self, name: &str) -> PoolSave {
        PoolSave {
            name: name.to_owned(),
            backstore: self.backstore.record(),
            flex_devs: self.thin_pool.record(),
            thinpool_dev: self.thin_pool.record(),
        }
    }
}

impl Pool for StratPool {
    fn create_filesystems<'a, 'b>(&'a mut self,
                                  pool_name: &str,
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
                .create_filesystem(pool_name, name, &dm, size)?;
            result.push((name, fs_uuid));
        }

        Ok(result)
    }

    fn add_blockdevs(&mut self,
                     pool_name: &str,
                     paths: &[&Path],
                     tier: BlockDevTier,
                     force: bool)
                     -> EngineResult<Vec<DevUuid>> {
        if tier == BlockDevTier::Cache {
            return Err(EngineError::Engine(ErrorEnum::Invalid, "UNIMPLEMENTED".into()));
        }

        let dm = DM::new()?;
        let bdev_info = self.backstore.add(&dm, paths, force)?;
        self.write_metadata(pool_name)?;
        Ok(bdev_info)
    }

    fn destroy(self) -> EngineResult<()> {
        let dm = DM::new()?;
        self.thin_pool.teardown(&dm)?;
        self.backstore.destroy(&dm)?;
        Ok(())
    }

    fn destroy_filesystems<'a>(&'a mut self,
                               pool_name: &str,
                               fs_uuids: &[FilesystemUuid])
                               -> EngineResult<Vec<FilesystemUuid>> {
        let dm = DM::new()?;

        let mut removed = Vec::new();
        for &uuid in fs_uuids {
            self.thin_pool.destroy_filesystem(&dm, pool_name, uuid)?;
            removed.push(uuid);
        }

        Ok(removed)
    }

    fn rename_filesystem(&mut self,
                         pool_name: &str,
                         uuid: FilesystemUuid,
                         new_name: &str)
                         -> EngineResult<RenameAction> {
        self.thin_pool
            .rename_filesystem(pool_name, uuid, new_name)
    }

    fn snapshot_filesystem(&mut self,
                           pool_name: &str,
                           origin_uuid: FilesystemUuid,
                           snapshot_name: &str)
                           -> EngineResult<FilesystemUuid> {
        self.thin_pool
            .snapshot_filesystem(&DM::new()?, pool_name, origin_uuid, snapshot_name)
    }

    fn total_physical_size(&self) -> Sectors {
        self.backstore.datadev_current_capacity()
    }

    fn total_physical_used(&self) -> EngineResult<Sectors> {
        self.thin_pool
            .total_physical_used()
            .and_then(|v| Ok(v + self.backstore.datadev_metadata_size()))
    }

    fn filesystems(&self) -> Vec<(Name, FilesystemUuid, &Filesystem)> {
        self.thin_pool.filesystems()
    }

    fn get_filesystem(&self, uuid: FilesystemUuid) -> Option<(Name, &Filesystem)> {
        self.thin_pool
            .get_filesystem_by_uuid(uuid)
            .map(|(name, fs)| (name, fs as &Filesystem))
    }

    fn get_mut_filesystem(&mut self, uuid: FilesystemUuid) -> Option<(Name, &mut Filesystem)> {
        self.thin_pool
            .get_mut_filesystem_by_uuid(uuid)
            .map(|(name, fs)| (name, fs as &mut Filesystem))
    }

    fn blockdevs(&self) -> Vec<(DevUuid, &BlockDev)> {
        self.backstore.blockdevs()
    }

    fn get_blockdev(&self, uuid: DevUuid) -> Option<(BlockDevTier, &BlockDev)> {
        self.backstore.get_blockdev_by_uuid(uuid)
    }

    fn get_mut_blockdev(&mut self, uuid: DevUuid) -> Option<(BlockDevTier, &mut BlockDev)> {
        self.backstore.get_mut_blockdev_by_uuid(uuid)
    }

    fn save_state(&mut self, pool_name: &str) -> EngineResult<()> {
        self.write_metadata(pool_name)
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::types::Redundancy;

    use super::super::backstore::find_all;
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
        let (uuid1, pool1) = StratPool::initialize(&dm, &name1, paths1, Redundancy::NONE, false)
            .unwrap();
        let metadata1 = pool1.record(name1);

        let name2 = "name2";
        let (uuid2, pool2) = StratPool::initialize(&dm, &name2, paths2, Redundancy::NONE, false)
            .unwrap();
        let metadata2 = pool2.record(name2);

        let pools = find_all().unwrap();
        assert_eq!(pools.len(), 2);
        let devnodes1 = pools.get(&uuid1).unwrap();
        let devnodes2 = pools.get(&uuid2).unwrap();
        let pool_save1 = get_metadata(uuid1, devnodes1).unwrap().unwrap();
        let pool_save2 = get_metadata(uuid2, devnodes2).unwrap().unwrap();
        assert_eq!(pool_save1, metadata1);
        assert_eq!(pool_save2, metadata2);

        pool1.teardown(&dm).unwrap();
        pool2.teardown(&dm).unwrap();
        let pools = find_all().unwrap();
        assert_eq!(pools.len(), 2);
        let devnodes1 = pools.get(&uuid1).unwrap();
        let devnodes2 = pools.get(&uuid2).unwrap();
        let pool_save1 = get_metadata(uuid1, devnodes1).unwrap().unwrap();
        let pool_save2 = get_metadata(uuid2, devnodes2).unwrap().unwrap();
        assert_eq!(pool_save1, metadata1);
        assert_eq!(pool_save2, metadata2);
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
        assert!(StratPool::initialize(&dm, "stratis_test_pool", paths, Redundancy::NONE, true)
                    .is_err());
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

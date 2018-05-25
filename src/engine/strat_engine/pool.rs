// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::vec::Vec;

use serde_json;
use uuid::Uuid;

use devicemapper::{Device, DmName, DmNameBuf, Sectors};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::engine::{BlockDev, Filesystem, Pool};
use super::super::types::{BlockDevTier, DevUuid, FilesystemUuid, Name, PoolUuid, Redundancy,
                          RenameAction};

use super::backstore::{Backstore, MIN_MDA_SECTORS};
use super::serde_structs::{PoolSave, Recordable};
use super::thinpool::{ThinPool, ThinPoolSizeParams};

pub use super::thinpool::{DATA_BLOCK_SIZE, DATA_LOWATER, INITIAL_DATA_SIZE};

/// Check the metadata of an individual pool for consistency.
pub fn check_metadata(metadata: &PoolSave) -> StratisResult<()> {
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
        let err_msg = format!(
            "{} used in thinpool, but {} given up by cache",
            total_allocated, metadata.backstore.next
        );
        Err(StratisError::Engine(ErrorEnum::Invalid, err_msg))
    } else {
        Ok(())
    }
}

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
    pub fn initialize(
        name: &str,
        paths: &[&Path],
        redundancy: Redundancy,
        force: bool,
    ) -> StratisResult<(PoolUuid, StratPool)> {
        let pool_uuid = Uuid::new_v4();

        let mut backstore = Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS, force)?;

        let thinpool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            DATA_LOWATER,
            &mut backstore,
        );
        let thinpool = match thinpool {
            Ok(thinpool) => thinpool,
            Err(err) => {
                let _ = backstore.destroy();
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
    pub fn setup(
        uuid: PoolUuid,
        devnodes: &HashMap<Device, PathBuf>,
        metadata: &PoolSave,
    ) -> StratisResult<(Name, StratPool)> {
        let backstore = Backstore::setup(uuid, &metadata.backstore, devnodes, None)?;
        let thinpool = ThinPool::setup(
            uuid,
            metadata.thinpool_dev.data_block_size,
            DATA_LOWATER,
            &metadata.flex_devs,
            &backstore,
        )?;

        Ok((
            Name::new(metadata.name.to_owned()),
            StratPool {
                backstore,
                redundancy: Redundancy::NONE,
                thin_pool: thinpool,
            },
        ))
    }

    /// Write current metadata to pool members.
    pub fn write_metadata(&mut self, name: &str) -> StratisResult<()> {
        let data = serde_json::to_string(&self.record(name))?;
        self.backstore.save_state(data.as_bytes())
    }

    /// Teardown a pool.
    #[cfg(test)]
    pub fn teardown(self) -> StratisResult<()> {
        self.thin_pool.teardown()?;
        self.backstore.teardown()
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
    pub fn event_on(&mut self, pool_name: &Name, dm_name: &DmName) -> StratisResult<()> {
        assert!(
            self.thin_pool
                .get_eventing_dev_names()
                .iter()
                .any(|x| dm_name == &**x)
        );
        if self.thin_pool.check(&mut self.backstore)? {
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
    fn create_filesystems<'a, 'b>(
        &'a mut self,
        pool_name: &str,
        specs: &[(&'b str, Option<Sectors>)],
    ) -> StratisResult<Vec<(&'b str, FilesystemUuid)>> {
        let names: HashMap<_, _> = HashMap::from_iter(specs.iter().map(|&tup| (tup.0, tup.1)));
        for name in names.keys() {
            if self.thin_pool.get_mut_filesystem_by_name(*name).is_some() {
                return Err(StratisError::Engine(
                    ErrorEnum::AlreadyExists,
                    name.to_string(),
                ));
            }
        }

        // TODO: Roll back on filesystem initialization failure.
        let mut result = Vec::new();
        for (name, size) in names {
            let fs_uuid = self.thin_pool.create_filesystem(pool_name, name, size)?;
            result.push((name, fs_uuid));
        }

        Ok(result)
    }

    fn add_blockdevs(
        &mut self,
        pool_name: &str,
        paths: &[&Path],
        tier: BlockDevTier,
        force: bool,
    ) -> StratisResult<Vec<DevUuid>> {
        self.thin_pool.suspend()?;
        let bdev_info = self.backstore.add_blockdevs(paths, tier, force)?;
        self.thin_pool.set_device(self.backstore.device())?;
        self.thin_pool.resume()?;
        self.write_metadata(pool_name)?;
        Ok(bdev_info)
    }

    fn destroy(self) -> StratisResult<()> {
        self.thin_pool.teardown()?;
        self.backstore.destroy()?;
        Ok(())
    }

    fn destroy_filesystems<'a>(
        &'a mut self,
        pool_name: &str,
        fs_uuids: &[FilesystemUuid],
    ) -> StratisResult<Vec<FilesystemUuid>> {
        let mut removed = Vec::new();
        for &uuid in fs_uuids {
            self.thin_pool.destroy_filesystem(pool_name, uuid)?;
            removed.push(uuid);
        }

        Ok(removed)
    }

    fn rename_filesystem(
        &mut self,
        pool_name: &str,
        uuid: FilesystemUuid,
        new_name: &str,
    ) -> StratisResult<RenameAction> {
        self.thin_pool.rename_filesystem(pool_name, uuid, new_name)
    }

    fn snapshot_filesystem(
        &mut self,
        pool_name: &str,
        origin_uuid: FilesystemUuid,
        snapshot_name: &str,
    ) -> StratisResult<FilesystemUuid> {
        self.thin_pool
            .snapshot_filesystem(pool_name, origin_uuid, snapshot_name)
    }

    fn total_physical_size(&self) -> Sectors {
        self.backstore.datatier_current_capacity()
    }

    fn total_physical_used(&self) -> StratisResult<Sectors> {
        self.thin_pool
            .total_physical_used()
            .and_then(|v| Ok(v + self.backstore.datatier_metadata_size()))
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
        self.backstore
            .blockdevs()
            .iter()
            .map(|&(u, b)| (u, b as &BlockDev))
            .collect()
    }

    fn get_blockdev(&self, uuid: DevUuid) -> Option<(BlockDevTier, &BlockDev)> {
        self.backstore
            .get_blockdev_by_uuid(uuid)
            .map(|(t, b)| (t, b as &BlockDev))
    }

    fn set_blockdev_user_info(
        &mut self,
        pool_name: &str,
        uuid: DevUuid,
        user_info: Option<&str>,
    ) -> StratisResult<bool> {
        if self.backstore.set_blockdev_user_info(uuid, user_info)? {
            self.write_metadata(pool_name)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs::OpenOptions;
    use std::io::{Read, Write};

    use nix::mount::{mount, umount, MsFlags};
    use tempfile;

    use super::super::super::types::Redundancy;

    use super::super::backstore::{find_all, get_metadata};
    use super::super::devlinks;
    use super::super::tests::{loopbacked, real};

    use super::*;

    fn invariant(pool: &StratPool, pool_name: &str) -> () {
        check_metadata(&pool.record(&Name::new(pool_name.into()))).unwrap();
    }

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

        let name1 = "name1";
        let (uuid1, pool1) =
            StratPool::initialize(&name1, paths1, Redundancy::NONE, false).unwrap();
        invariant(&pool1, &name1);

        let metadata1 = pool1.record(name1);

        let name2 = "name2";
        let (uuid2, pool2) =
            StratPool::initialize(&name2, paths2, Redundancy::NONE, false).unwrap();
        invariant(&pool2, &name2);

        let metadata2 = pool2.record(name2);

        let pools = find_all().unwrap();
        assert_eq!(pools.len(), 2);
        let devnodes1 = pools.get(&uuid1).unwrap();
        let devnodes2 = pools.get(&uuid2).unwrap();
        let pool_save1 = get_metadata(uuid1, devnodes1).unwrap().unwrap();
        let pool_save2 = get_metadata(uuid2, devnodes2).unwrap().unwrap();
        assert_eq!(pool_save1, metadata1);
        assert_eq!(pool_save2, metadata2);

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
    }

    #[test]
    pub fn loop_test_basic_metadata() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(2, 3, None),
            test_basic_metadata,
        );
    }

    #[test]
    pub fn real_test_basic_metadata() {
        real::test_with_spec(
            real::DeviceLimits::AtLeast(2, None, None),
            test_basic_metadata,
        );
    }

    /// Verify that a pool with no devices does not have the minimum amount of
    /// space required.
    fn test_empty_pool(paths: &[&Path]) -> () {
        assert_eq!(paths.len(), 0);
        assert!(StratPool::initialize("stratis_test_pool", paths, Redundancy::NONE, true).is_err());
    }

    #[test]
    pub fn loop_test_empty_pool() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Exactly(0, None), test_empty_pool);
    }

    #[test]
    pub fn real_test_empty_pool() {
        real::test_with_spec(real::DeviceLimits::Exactly(0, None, None), test_empty_pool);
    }

    /// Test that adding a cachedev causes metadata to be updated.
    /// Verify that teardown and setup of pool allows reading from filesystem
    /// written before cache was added. Check some basic facts about the
    /// metadata.
    fn test_add_cachedevs(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let name = "stratis-test-pool";
        devlinks::setup_devlinks(Vec::new().into_iter()).unwrap();
        let (uuid, mut pool) =
            StratPool::initialize(&name, paths2, Redundancy::NONE, false).unwrap();
        devlinks::pool_added(&name).unwrap();
        invariant(&pool, &name);

        let metadata1 = pool.record(name);
        assert!(metadata1.backstore.cache_devs.is_none());
        assert!(metadata1.backstore.cache_segments.is_none());
        assert!(metadata1.backstore.meta_segments.is_none());

        let (_, fs_uuid) = pool.create_filesystems(&name, &[("stratis-filesystem", None)])
            .unwrap()
            .pop()
            .unwrap();
        invariant(&pool, &name);

        let tmp_dir = tempfile::Builder::new()
            .prefix("stratis_testing")
            .tempdir()
            .unwrap();
        let new_file = tmp_dir.path().join("stratis_test.txt");
        let bytestring = b"some bytes";
        {
            let (_, fs) = pool.get_filesystem(fs_uuid).unwrap();
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

        pool.add_blockdevs(&name, paths1, BlockDevTier::Cache, false)
            .unwrap();
        invariant(&pool, &name);

        let metadata2 = pool.record(name);
        assert!(metadata2.backstore.cache_devs.is_some());
        assert!(metadata2.backstore.cache_segments.is_some());
        assert!(metadata2.backstore.meta_segments.is_some());

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

        umount(tmp_dir.path()).unwrap();

        pool.teardown().unwrap();

        let pools = find_all().unwrap();
        assert_eq!(pools.len(), 1);
        let devices = pools.get(&uuid).unwrap();
        let (name, pool) = StratPool::setup(
            uuid,
            &devices,
            &get_metadata(uuid, &devices).unwrap().unwrap(),
        ).unwrap();
        invariant(&pool, &name);

        let metadata3 = pool.record(&name);

        // FIXME: A simple test of equality between metadata2 and metadata3
        // should be all that is required once blockdevs maintain a consistent
        // order across teardown/setup operations.
        assert!(metadata3.backstore.cache_devs.is_some());
        assert!(metadata3.backstore.cache_segments.is_some());
        assert!(metadata3.backstore.meta_segments.is_some());

        assert_eq!(
            metadata3
                .backstore
                .cache_devs
                .as_ref()
                .map(|bds| bds.iter().map(|bd| bd.uuid).collect::<HashSet<_>>()),
            metadata2
                .backstore
                .cache_devs
                .as_ref()
                .map(|bds| bds.iter().map(|bd| bd.uuid).collect::<HashSet<_>>())
        );
        assert_eq!(
            metadata3
                .backstore
                .data_devs
                .iter()
                .map(|bd| bd.uuid)
                .collect::<HashSet<_>>(),
            metadata2
                .backstore
                .data_devs
                .iter()
                .map(|bd| bd.uuid)
                .collect::<HashSet<_>>()
        );

        let mut buf = [0u8; 10];
        {
            let (_, fs) = pool.get_filesystem(fs_uuid).unwrap();
            mount(
                Some(&fs.devnode()),
                tmp_dir.path(),
                Some("xfs"),
                MsFlags::empty(),
                None as Option<&str>,
            ).unwrap();
            OpenOptions::new()
                .read(true)
                .open(&new_file)
                .unwrap()
                .read(&mut buf)
                .unwrap();
        }
        assert_eq!(&buf, bytestring);
        umount(tmp_dir.path()).unwrap();
    }

    #[test]
    pub fn loop_test_add_cachedevs() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(2, 3, None),
            test_add_cachedevs,
        );
    }

    #[test]
    pub fn real_test_add_cachedevs() {
        real::test_with_spec(
            real::DeviceLimits::AtLeast(2, None, None),
            test_add_cachedevs,
        );
    }
}

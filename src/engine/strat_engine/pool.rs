// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::HashMap,
    iter::FromIterator,
    path::{Path, PathBuf},
    vec::Vec,
};

use chrono::{DateTime, Utc};
use serde_json;
use uuid::Uuid;

use devicemapper::{Device, DmName, DmNameBuf, Sectors};

use crate::{
    engine::{
        engine::{BlockDev, Filesystem, Pool},
        shared::init_cache_idempotent_or_err,
        strat_engine::{
            backstore::{Backstore, MDADataSize},
            names::validate_name,
            serde_structs::{FlexDevsSave, PoolSave, Recordable},
            thinpool::{ThinPool, ThinPoolSizeParams, DATA_BLOCK_SIZE},
        },
        types::{
            BlockDevTier, CreateAction, DevUuid, FilesystemUuid, MaybeDbusPath, Name, PoolUuid,
            Redundancy, RenameAction, SetCreateAction, SetDeleteAction,
        },
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

/// Get the index which indicates the start of unallocated space in the cap
/// device.
/// NOTE: Since segments are always allocated to each flex dev in order, the
/// last segment for each is the highest. This allows avoiding sorting all the
/// segments and just sorting the set consisting of the last segment from
/// each list of segments.
/// Precondition: This method is called only when setting up a pool, which
/// ensures that the flex devs metadata lists are all non-empty.
fn next_index(flex_devs: &FlexDevsSave) -> Sectors {
    let expect_msg = "Setting up rather than initializing a pool, so each flex dev must have been allocated at least some segments.";
    [
        flex_devs
            .meta_dev
            .last()
            .unwrap_or_else(|| panic!(expect_msg)),
        flex_devs
            .thin_meta_dev
            .last()
            .unwrap_or_else(|| panic!(expect_msg)),
        flex_devs
            .thin_data_dev
            .last()
            .unwrap_or_else(|| panic!(expect_msg)),
        flex_devs
            .thin_meta_dev_spare
            .last()
            .unwrap_or_else(|| panic!(expect_msg)),
    ]
    .iter()
    .max_by_key(|x| x.0)
    .map(|&&(start, length)| start + length)
    .expect("iterator is non-empty")
}

/// Check the metadata of an individual pool for consistency.
/// Precondition: This method is called only when setting up a pool, which
/// ensures that the flex devs metadata lists are all non-empty.
fn check_metadata(metadata: &PoolSave) -> StratisResult<()> {
    let flex_devs = &metadata.flex_devs;
    let next = next_index(flex_devs);
    let allocated_from_cap = metadata.backstore.cap.allocs[0].1;

    if allocated_from_cap != next {
        let err_msg = format!(
            "{} used in thinpool, but {} allocated from backstore cap device",
            next, allocated_from_cap
        );
        return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
    }

    // If the total length of the allocations in the flex devs, does not
    // equal next, consider the situation an error.
    {
        let total_allocated = flex_devs
            .meta_dev
            .iter()
            .chain(flex_devs.thin_meta_dev.iter())
            .chain(flex_devs.thin_data_dev.iter())
            .chain(flex_devs.thin_meta_dev_spare.iter())
            .map(|x| x.1)
            .sum::<Sectors>();
        if total_allocated != next {
            let err_msg = format!(
                "{} used in thinpool, but {} given up by cache for pool {}",
                total_allocated, next, metadata.name
            );
            return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
        }
    }

    // If the amount allocated to the cap device is less than the amount
    // allocated to the flex devices, consider the situation an error.
    // Consider it an error if the amount allocated to the cap device is 0.
    // If this is the case, then the thin pool can not exist.
    {
        let total_allocated = metadata.backstore.data_tier.blockdev.allocs[0]
            .iter()
            .map(|x| x.length)
            .sum::<Sectors>();

        if total_allocated == Sectors(0) {
            let err_msg = format!(
                "no segments allocated to the cap device for pool {}",
                metadata.name
            );
            return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
        }

        if next > total_allocated {
            let err_msg = format!(
                "{} allocated to cap device, but {} allocated to flex devs",
                next, total_allocated
            );
            return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
        }
    }

    Ok(())
}

#[derive(Debug)]
pub struct StratPool {
    backstore: Backstore,
    redundancy: Redundancy,
    thin_pool: ThinPool,
    dbus_path: MaybeDbusPath,
}

impl StratPool {
    /// Initialize a Stratis Pool.
    /// 1. Initialize the block devices specified by paths.
    /// 2. Set up thinpool device to back filesystems.
    pub fn initialize(
        name: &str,
        paths: &[&Path],
        redundancy: Redundancy,
        keyfile_path: Option<PathBuf>,
    ) -> StratisResult<(PoolUuid, StratPool)> {
        let pool_uuid = Uuid::new_v4();

        // FIXME: Initializing with the minimum MDA size is not necessarily
        // enough. If there are enough devices specified, more space will be
        // required.
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MDADataSize::default(), keyfile_path)?;

        let thinpool = ThinPool::new(
            pool_uuid,
            &ThinPoolSizeParams::default(),
            DATA_BLOCK_SIZE,
            &mut backstore,
        );

        let mut thinpool = match thinpool {
            Ok(thinpool) => thinpool,
            Err(err) => {
                let _ = backstore.destroy();
                return Err(err);
            }
        };

        thinpool.check(pool_uuid, &mut backstore)?;

        let mut pool = StratPool {
            backstore,
            redundancy,
            thin_pool: thinpool,
            dbus_path: MaybeDbusPath(None),
        };

        pool.write_metadata(&Name::new(name.to_owned()))?;

        Ok((pool_uuid, pool))
    }

    /// Setup a StratPool using its UUID and the list of devnodes it has.
    /// Precondition: every device in devnodes has already been determined
    /// to belong to the pool with the specified uuid.
    /// Precondition: A metadata verification step has already been run.
    /// Precondition: All blockdevs have been encrypted using the same keyfile
    /// in the same pool.
    pub fn setup(
        uuid: PoolUuid,
        devnodes: &HashMap<Device, PathBuf>,
        timestamp: DateTime<Utc>,
        metadata: &PoolSave,
    ) -> StratisResult<(Name, StratPool)> {
        check_metadata(metadata)?;

        let mut backstore = Backstore::setup(uuid, &metadata.backstore, devnodes, timestamp)?;
        let mut thinpool = ThinPool::setup(
            uuid,
            &metadata.thinpool_dev,
            &metadata.flex_devs,
            &backstore,
        )?;

        let changed = thinpool.check(uuid, &mut backstore)?;

        let mut pool = StratPool {
            backstore,
            redundancy: Redundancy::NONE,
            thin_pool: thinpool,
            dbus_path: MaybeDbusPath(None),
        };

        let pool_name = &metadata.name;

        if changed {
            pool.write_metadata(pool_name)?;
        }

        Ok((Name::new(pool_name.to_owned()), pool))
    }

    /// Write current metadata to pool members.
    pub fn write_metadata(&mut self, name: &str) -> StratisResult<()> {
        let data = serde_json::to_string(&self.record(name))?;
        self.backstore.save_state(data.as_bytes())
    }

    /// Teardown a pool.
    #[cfg(test)]
    pub fn teardown(&mut self) -> StratisResult<()> {
        self.thin_pool.teardown()?;
        self.backstore.teardown()
    }

    pub fn has_filesystems(&self) -> bool {
        self.thin_pool.has_filesystems()
    }

    /// The names of DM devices belonging to this pool that may generate events
    pub fn get_eventing_dev_names(&self, pool_uuid: PoolUuid) -> Vec<DmNameBuf> {
        self.thin_pool.get_eventing_dev_names(pool_uuid)
    }

    /// Called when a DM device in this pool has generated an event.
    // TODO: Just check the device that evented. Currently checks
    // everything.
    pub fn event_on(
        &mut self,
        pool_uuid: PoolUuid,
        pool_name: &Name,
        dm_name: &DmName,
    ) -> StratisResult<()> {
        assert!(self
            .thin_pool
            .get_eventing_dev_names(pool_uuid)
            .iter()
            .any(|x| dm_name == &**x));
        if self.thin_pool.check(pool_uuid, &mut self.backstore)? {
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

    fn datadevs_encrypted(&self) -> bool {
        self.backstore.data_tier_is_encrypted()
    }
}

impl Pool for StratPool {
    fn init_cache(
        &mut self,
        pool_uuid: PoolUuid,
        pool_name: &str,
        blockdevs: &[&Path],
    ) -> StratisResult<SetCreateAction<DevUuid>> {
        if self.is_encrypted() {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                "Use of a cache is not supported with an encrypted pool".to_string(),
            ));
        }
        if self.backstore.cache_initialized() {
            init_cache_idempotent_or_err(
                blockdevs,
                self.backstore
                    .cachedevs()
                    .into_iter()
                    .map(|(_, bd)| bd.devnode()),
            )
        } else {
            // If adding cache devices, must suspend the pool, since the cache
            // must be augmented with the new devices.
            self.thin_pool.suspend()?;
            let devices_result = self.backstore.init_cache(pool_uuid, blockdevs);
            self.thin_pool.resume()?;
            let devices = devices_result?;
            self.write_metadata(pool_name)?;
            Ok(SetCreateAction::new(devices))
        }
    }

    fn create_filesystems<'a, 'b>(
        &'a mut self,
        pool_uuid: PoolUuid,
        pool_name: &str,
        specs: &[(&'b str, Option<Sectors>)],
    ) -> StratisResult<SetCreateAction<(&'b str, FilesystemUuid)>> {
        let names: HashMap<_, _> = HashMap::from_iter(specs.iter().map(|&tup| (tup.0, tup.1)));

        names.iter().fold(Ok(()), |res, (name, _)| {
            res.and_then(|()| validate_name(name))
        })?;

        // TODO: Roll back on filesystem initialization failure.
        let mut result = Vec::new();
        for (name, size) in names {
            if self.thin_pool.get_mut_filesystem_by_name(name).is_none() {
                let fs_uuid = self
                    .thin_pool
                    .create_filesystem(pool_uuid, pool_name, name, size)?;
                result.push((name, fs_uuid));
            }
        }

        Ok(SetCreateAction::new(result))
    }

    fn add_blockdevs(
        &mut self,
        pool_uuid: PoolUuid,
        pool_name: &str,
        paths: &[&Path],
        tier: BlockDevTier,
    ) -> StratisResult<SetCreateAction<DevUuid>> {
        let bdev_info = if tier == BlockDevTier::Cache {
            if self.backstore.cache_initialized() {
                // If adding cache devices, must suspend the pool, since the cache
                // must be augmented with the new devices.
                self.thin_pool.suspend()?;
                let bdev_info_res =
                    self.backstore
                        .add_cachedevs(pool_uuid, paths)
                        .and_then(|bdi| {
                            self.thin_pool
                                .set_device(self.backstore.device().expect(
                                    "Since thin pool exists, space must have been allocated \
                                     from the backstore, so backstore must have a cap device",
                                ))
                                .and(Ok(bdi))
                        });
                self.thin_pool.resume()?;
                let bdev_info = bdev_info_res?;
                Ok(SetCreateAction::new(bdev_info))
            } else {
                Err(StratisError::Error(
                    "The cache has not yet been initialized".to_string(),
                ))
            }
        } else {
            // If just adding data devices, no need to suspend the pool.
            // No action will be taken on the DM devices.
            let bdev_info = self.backstore.add_datadevs(pool_uuid, paths)?;

            // Adding data devices does not change the state of the thin
            // pool at all. However, if the thin pool is in a state
            // where it would request an allocation from the backstore the
            // addition of the new data devs may have changed its context
            // so that it can satisfy the allocation request where
            // previously it could not. Run check() in case that is true.
            self.thin_pool.check(pool_uuid, &mut self.backstore)?;
            Ok(SetCreateAction::new(bdev_info))
        };
        self.write_metadata(pool_name)?;
        bdev_info
    }

    fn destroy(&mut self) -> StratisResult<()> {
        self.thin_pool.teardown()?;
        self.backstore.destroy()?;
        Ok(())
    }

    fn destroy_filesystems<'a>(
        &'a mut self,
        pool_name: &str,
        fs_uuids: &[FilesystemUuid],
    ) -> StratisResult<SetDeleteAction<FilesystemUuid>> {
        let mut removed = Vec::new();
        for &uuid in fs_uuids {
            if let Some(uuid) = self.thin_pool.destroy_filesystem(pool_name, uuid)? {
                removed.push(uuid);
            }
        }

        Ok(SetDeleteAction::new(removed))
    }

    fn rename_filesystem(
        &mut self,
        pool_name: &str,
        uuid: FilesystemUuid,
        new_name: &str,
    ) -> StratisResult<RenameAction<FilesystemUuid>> {
        validate_name(new_name)?;
        match self.thin_pool.rename_filesystem(pool_name, uuid, new_name) {
            Ok(Some(uuid)) => Ok(RenameAction::Renamed(uuid)),
            Ok(None) => Ok(RenameAction::Identity),
            Err(e) => {
                if let StratisError::Engine(ErrorEnum::NotFound, _) = e {
                    Ok(RenameAction::NoSource)
                } else {
                    Err(e)
                }
            }
        }
    }

    fn snapshot_filesystem(
        &mut self,
        pool_uuid: PoolUuid,
        pool_name: &str,
        origin_uuid: FilesystemUuid,
        snapshot_name: &str,
    ) -> StratisResult<CreateAction<(FilesystemUuid, &mut dyn Filesystem)>> {
        validate_name(snapshot_name)?;

        if self
            .thin_pool
            .get_filesystem_by_name(snapshot_name)
            .is_some()
        {
            return Ok(CreateAction::Identity);
        }

        self.thin_pool
            .snapshot_filesystem(pool_uuid, pool_name, origin_uuid, snapshot_name)
            .map(CreateAction::Created)
    }

    fn total_physical_size(&self) -> Sectors {
        self.backstore.datatier_size()
    }

    fn total_physical_used(&self) -> StratisResult<Sectors> {
        // TODO: note that with the addition of another layer, the
        // calculation of the amount of physical spaced used by means
        // of adding the amount used by Stratis metadata to the amount used
        // by the pool abstraction will be invalid. In the event of, e.g.,
        // software RAID, the amount will be far too low to be useful, in the
        // event of, e.g, VDO, the amount will be far too large to be useful.
        self.thin_pool
            .total_physical_used()
            .map(|v| v + self.backstore.datatier_metadata_size())
    }

    fn filesystems(&self) -> Vec<(Name, FilesystemUuid, &dyn Filesystem)> {
        self.thin_pool.filesystems()
    }

    fn filesystems_mut(&mut self) -> Vec<(Name, FilesystemUuid, &mut dyn Filesystem)> {
        self.thin_pool.filesystems_mut()
    }

    fn get_filesystem(&self, uuid: FilesystemUuid) -> Option<(Name, &dyn Filesystem)> {
        self.thin_pool
            .get_filesystem_by_uuid(uuid)
            .map(|(name, fs)| (name, fs as &dyn Filesystem))
    }

    fn get_mut_filesystem(&mut self, uuid: FilesystemUuid) -> Option<(Name, &mut dyn Filesystem)> {
        self.thin_pool
            .get_mut_filesystem_by_uuid(uuid)
            .map(|(name, fs)| (name, fs as &mut dyn Filesystem))
    }

    fn blockdevs(&self) -> Vec<(DevUuid, &dyn BlockDev)> {
        self.backstore
            .blockdevs()
            .iter()
            .map(|&(u, b)| (u, b as &dyn BlockDev))
            .collect()
    }

    fn blockdevs_mut(&mut self) -> Vec<(DevUuid, &mut dyn BlockDev)> {
        self.backstore
            .blockdevs_mut()
            .into_iter()
            .map(|(u, b)| (u, b as &mut dyn BlockDev))
            .collect()
    }

    fn get_blockdev(&self, uuid: DevUuid) -> Option<(BlockDevTier, &dyn BlockDev)> {
        self.backstore
            .get_blockdev_by_uuid(uuid)
            .map(|(t, b)| (t, b as &dyn BlockDev))
    }

    fn get_mut_blockdev(&mut self, uuid: DevUuid) -> Option<(BlockDevTier, &mut dyn BlockDev)> {
        self.backstore
            .get_mut_blockdev_by_uuid(uuid)
            .map(|(t, b)| (t, b as &mut dyn BlockDev))
    }

    fn set_blockdev_user_info(
        &mut self,
        pool_name: &str,
        uuid: DevUuid,
        user_info: Option<&str>,
    ) -> StratisResult<RenameAction<DevUuid>> {
        let result = self.backstore.set_blockdev_user_info(uuid, user_info);
        match result {
            Ok(Some(uuid)) => {
                self.write_metadata(pool_name)?;
                Ok(RenameAction::Renamed(uuid))
            }
            Ok(None) => Ok(RenameAction::Identity),
            Err(_) => Ok(RenameAction::NoSource),
        }
    }

    fn set_dbus_path(&mut self, path: MaybeDbusPath) {
        self.thin_pool.set_dbus_path(path.clone());
        self.dbus_path = path
    }

    fn get_dbus_path(&self) -> &MaybeDbusPath {
        &self.dbus_path
    }

    fn is_encrypted(&self) -> bool {
        self.datadevs_encrypted()
    }

    fn cache_initialized(&self) -> bool {
        self.backstore.cache_initialized()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::OpenOptions,
        io::{BufWriter, Read, Write},
    };

    use nix::mount::{mount, umount, MsFlags};
    use tempfile;

    use devicemapper::{Bytes, IEC, SECTOR_SIZE};

    use crate::engine::{
        strat_engine::{
            backstore::{find_all, get_metadata},
            cmd, devlinks,
            tests::{loopbacked, real},
        },
        types::{EngineAction, PoolExtendState, PoolState, Redundancy},
    };

    use super::*;

    fn invariant(pool: &StratPool, pool_name: &str) {
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
        let (uuid1, mut pool1) =
            StratPool::initialize(name1, paths1, Redundancy::NONE, None).unwrap();
        invariant(&pool1, name1);

        let metadata1 = pool1.record(name1);

        let name2 = "name2";
        let (uuid2, mut pool2) =
            StratPool::initialize(name2, paths2, Redundancy::NONE, None).unwrap();
        invariant(&pool2, name2);

        let metadata2 = pool2.record(name2);

        cmd::udev_settle().unwrap();
        let pools = find_all().unwrap();
        assert_eq!(pools.len(), 2);
        let devnodes1 = &pools[&uuid1];
        let devnodes2 = &pools[&uuid2];
        let (_, pool_save1) = get_metadata(uuid1, devnodes1).unwrap().unwrap();
        let (_, pool_save2) = get_metadata(uuid2, devnodes2).unwrap().unwrap();
        assert_eq!(pool_save1, metadata1);
        assert_eq!(pool_save2, metadata2);

        pool1.teardown().unwrap();
        pool2.teardown().unwrap();

        cmd::udev_settle().unwrap();
        let pools = find_all().unwrap();
        assert_eq!(pools.len(), 2);
        let devnodes1 = &pools[&uuid1];
        let devnodes2 = &pools[&uuid2];
        let (_, pool_save1) = get_metadata(uuid1, devnodes1).unwrap().unwrap();
        let (_, pool_save2) = get_metadata(uuid2, devnodes2).unwrap().unwrap();
        assert_eq!(pool_save1, metadata1);
        assert_eq!(pool_save2, metadata2);
    }

    #[test]
    pub fn loop_test_basic_metadata() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_basic_metadata,
        );
    }

    #[test]
    pub fn real_test_basic_metadata() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_basic_metadata,
        );
    }

    /// Verify that a pool with no devices does not have the minimum amount of
    /// space required.
    fn test_empty_pool(paths: &[&Path]) {
        assert_eq!(paths.len(), 0);
        assert_matches!(
            StratPool::initialize("stratis_test_pool", paths, Redundancy::NONE, None),
            Err(_)
        );
    }

    #[test]
    pub fn loop_test_empty_pool() {
        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Exactly(0, None), test_empty_pool);
    }

    #[test]
    pub fn real_test_empty_pool() {
        real::test_with_spec(&real::DeviceLimits::Exactly(0, None, None), test_empty_pool);
    }

    /// Test that adding a cachedev causes metadata to be updated.
    /// Verify that teardown and setup of pool allows reading from filesystem
    /// written before cache was added. Check some basic facts about the
    /// metadata.
    fn test_add_cachedevs(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let name = "stratis-test-pool";
        devlinks::cleanup_devlinks(Vec::new().into_iter());
        let (uuid, mut pool) = StratPool::initialize(name, paths2, Redundancy::NONE, None).unwrap();
        devlinks::pool_added(name);
        invariant(&pool, name);

        let metadata1 = pool.record(name);
        assert_matches!(metadata1.backstore.cache_tier, None);

        let (_, fs_uuid) = pool
            .create_filesystems(uuid, name, &[("stratis-filesystem", None)])
            .unwrap()
            .changed()
            .and_then(|mut fs| fs.pop())
            .unwrap();
        invariant(&pool, name);

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
            )
            .unwrap();
            OpenOptions::new()
                .create(true)
                .write(true)
                .open(&new_file)
                .unwrap()
                .write_all(bytestring)
                .unwrap();
        }

        pool.init_cache(uuid, name, paths1).unwrap();
        invariant(&pool, name);

        let metadata2 = pool.record(name);
        assert!(metadata2.backstore.cache_tier.is_some());

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

        umount(tmp_dir.path()).unwrap();

        pool.teardown().unwrap();

        cmd::udev_settle().unwrap();
        let pools = find_all().unwrap();
        assert_eq!(pools.len(), 1);
        let devices = &pools[&uuid];

        let (timestamp, metadata) = get_metadata(uuid, devices).unwrap().unwrap();
        let (name, pool) = StratPool::setup(uuid, devices, timestamp, &metadata).unwrap();
        invariant(&pool, &name);

        let mut buf = [0u8; 10];
        {
            let (_, fs) = pool.get_filesystem(fs_uuid).unwrap();
            mount(
                Some(&fs.devnode()),
                tmp_dir.path(),
                Some("xfs"),
                MsFlags::empty(),
                None as Option<&str>,
            )
            .unwrap();
            OpenOptions::new()
                .read(true)
                .open(&new_file)
                .unwrap()
                .read_exact(&mut buf)
                .unwrap();
        }
        assert_eq!(&buf, bytestring);
        umount(tmp_dir.path()).unwrap();
    }

    #[test]
    pub fn loop_test_add_cachedevs() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_add_cachedevs,
        );
    }

    #[test]
    pub fn real_test_add_cachedevs() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_add_cachedevs,
        );
    }

    /// Verify that adding additional blockdevs will cause a pool that is
    /// out of space to be extended.
    fn test_add_datadevs(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let (paths1, paths2) = paths.split_at(1);

        let name = "stratis-test-pool";
        devlinks::cleanup_devlinks(Vec::new().into_iter());
        let (pool_uuid, mut pool) =
            StratPool::initialize(name, paths1, Redundancy::NONE, None).unwrap();
        devlinks::pool_added(name);
        invariant(&pool, name);

        let fs_name = "stratis_test_filesystem";
        let (_, fs_uuid) = pool
            .create_filesystems(pool_uuid, name, &[(fs_name, None)])
            .unwrap()
            .changed()
            .and_then(|mut fs| fs.pop())
            .expect("just created one");

        let devnode = pool.get_filesystem(fs_uuid).unwrap().1.devnode();

        {
            let buffer_length = IEC::Mi;
            let mut f = BufWriter::with_capacity(
                buffer_length as usize,
                OpenOptions::new().write(true).open(devnode).unwrap(),
            );

            let buf = &[1u8; SECTOR_SIZE];

            let mut amount_written = Sectors(0);
            let buffer_length = Bytes(buffer_length).sectors();
            while pool.thin_pool.extend_state() == PoolExtendState::Good
                && pool.thin_pool.state() == PoolState::Running
            {
                f.write_all(buf).unwrap();
                amount_written += Sectors(1);
                // Run check roughly every time the buffer is cleared.
                // Running it more often is pointless as the pool is guaranteed
                // not to see any effects unless the buffer is cleared.
                if amount_written % buffer_length == Sectors(1) {
                    pool.thin_pool
                        .check(pool_uuid, &mut pool.backstore)
                        .unwrap();
                }
            }

            pool.add_blockdevs(pool_uuid, name, paths2, BlockDevTier::Data)
                .unwrap();
            assert_matches!(pool.thin_pool.extend_state(), PoolExtendState::Good);
            assert_matches!(pool.thin_pool.state(), PoolState::Running);
        }
    }

    #[test]
    pub fn loop_test_add_datadevs() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, Some((4u64 * Bytes(IEC::Gi)).sectors())),
            test_add_datadevs,
        );
    }

    #[test]
    pub fn real_test_add_datadevs() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(
                2,
                Some((2u64 * Bytes(IEC::Gi)).sectors()),
                Some((4u64 * Bytes(IEC::Gi)).sectors()),
            ),
            test_add_datadevs,
        );
    }
}

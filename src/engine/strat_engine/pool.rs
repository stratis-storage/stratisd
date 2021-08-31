// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{path::Path, vec::Vec};

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use devicemapper::{Bytes, DmNameBuf, Sectors};
use stratisd_proc_macros::strat_pool_impl_gen;

use crate::{
    engine::{
        engine::Pool,
        shared::{
            init_cache_idempotent_or_err, validate_filesystem_size_specs, validate_name,
            validate_paths,
        },
        strat_engine::{
            backstore::{Backstore, StratBlockDev},
            metadata::MDADataSize,
            serde_structs::{FlexDevsSave, PoolSave, Recordable},
            thinpool::{StratFilesystem, ThinPool, ThinPoolSizeParams, DATA_BLOCK_SIZE},
        },
        types::{
            ActionAvailability, BlockDevTier, Clevis, CreateAction, DeleteAction, DevUuid,
            EncryptionInfo, FilesystemUuid, Key, KeyDescription, Name, PoolEncryptionInfo,
            PoolUuid, Redundancy, RegenAction, RenameAction, SetCreateAction, SetDeleteAction,
        },
    },
    stratis::{StratisError, StratisResult},
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
            .unwrap_or_else(|| panic!("{}", expect_msg)),
        flex_devs
            .thin_meta_dev
            .last()
            .unwrap_or_else(|| panic!("{}", expect_msg)),
        flex_devs
            .thin_data_dev
            .last()
            .unwrap_or_else(|| panic!("{}", expect_msg)),
        flex_devs
            .thin_meta_dev_spare
            .last()
            .unwrap_or_else(|| panic!("{}", expect_msg)),
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
        return Err(StratisError::Msg(err_msg));
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
            return Err(StratisError::Msg(err_msg));
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
            return Err(StratisError::Msg(err_msg));
        }

        if next > total_allocated {
            let err_msg = format!(
                "{} allocated to cap device, but {} allocated to flex devs",
                next, total_allocated
            );
            return Err(StratisError::Msg(err_msg));
        }
    }

    Ok(())
}

#[derive(Debug)]
pub struct StratPool {
    backstore: Backstore,
    redundancy: Redundancy,
    thin_pool: ThinPool,
    action_avail: ActionAvailability,
}

#[strat_pool_impl_gen]
impl StratPool {
    /// Initialize a Stratis Pool.
    /// 1. Initialize the block devices specified by paths.
    /// 2. Set up thinpool device to back filesystems.
    /// Precondition: p.is_absolute() is true for all p in paths
    pub fn initialize(
        name: &str,
        paths: &[&Path],
        redundancy: Redundancy,
        encryption_info: Option<&EncryptionInfo>,
    ) -> StratisResult<(PoolUuid, StratPool)> {
        let pool_uuid = PoolUuid::new_v4();

        // FIXME: Initializing with the minimum MDA size is not necessarily
        // enough. If there are enough devices specified, more space will be
        // required.
        let mut backstore =
            Backstore::initialize(pool_uuid, paths, MDADataSize::default(), encryption_info)?;

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
            action_avail: ActionAvailability::Full,
        };

        pool.write_metadata(&Name::new(name.to_owned()))?;

        Ok((pool_uuid, pool))
    }

    /// Setup a StratPool using its UUID and the list of devnodes it has.
    /// Precondition: every device in devnodes has already been determined
    /// to belong to the pool with the specified uuid.
    /// Precondition: A metadata verification step has already been run.
    ///
    /// Precondition:
    ///   * key_description.is_some() -> every StratBlockDev in datadevs has a
    ///   key description and that key description == key_description
    ///   * key_description.is_none() -> no StratBlockDev in datadevs has a
    ///   key description.
    ///   * no StratBlockDev in cachdevs has a key description
    pub fn setup(
        uuid: PoolUuid,
        datadevs: Vec<StratBlockDev>,
        cachedevs: Vec<StratBlockDev>,
        timestamp: DateTime<Utc>,
        metadata: &PoolSave,
        action_avail: ActionAvailability,
    ) -> StratisResult<(Name, StratPool)> {
        check_metadata(metadata)?;

        let mut backstore =
            Backstore::setup(uuid, &metadata.backstore, datadevs, cachedevs, timestamp)?;
        let pool_name = &metadata.name;

        let mut thinpool = ThinPool::setup(
            pool_name,
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
            action_avail,
        };

        if changed {
            pool.write_metadata(pool_name)?;
        }

        Ok((Name::new(pool_name.to_owned()), pool))
    }

    /// Send a synthetic udev change event to every filesystem on the given pool.
    pub fn udev_pool_change(&self, pool_name: &str) {
        for (name, uuid, fs) in self.thin_pool.filesystems() {
            fs.udev_fs_change(pool_name, uuid, &name);
        }
    }

    /// Write current metadata to pool members.
    #[pool_mutating_action("NoRequests")]
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
    #[pool_mutating_action("NoPoolChanges")]
    pub fn event_on(&mut self, pool_uuid: PoolUuid, pool_name: &Name) -> StratisResult<()> {
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

    pub fn get_strat_blockdev(&self, uuid: DevUuid) -> Option<(BlockDevTier, &StratBlockDev)> {
        self.backstore.get_blockdev_by_uuid(uuid)
    }

    #[pool_mutating_action("NoRequests")]
    pub fn get_mut_strat_blockdev(
        &mut self,
        uuid: DevUuid,
    ) -> StratisResult<Option<(BlockDevTier, &mut StratBlockDev)>> {
        Ok(self.backstore.get_mut_blockdev_by_uuid(uuid))
    }

    /// Destroy the pool.
    /// Precondition: All filesystems belonging to this pool must be
    /// unmounted.
    ///
    /// This method is not a mutating action as the pool should be allowed
    /// to be destroyed even if the metadata is inconsistent.
    pub fn destroy(&mut self) -> StratisResult<()> {
        self.thin_pool.teardown()?;
        self.backstore.destroy()?;
        Ok(())
    }

    #[cfg(test)]
    #[pool_mutating_action("NoRequests")]
    #[pool_rollback]
    pub fn return_rollback_failure(&mut self) -> StratisResult<()> {
        Err(StratisError::RollbackError {
            causal_error: Box::new(StratisError::Msg("Causal error".to_string())),
            rollback_error: Box::new(StratisError::Msg("Rollback error".to_string())),
        })
    }

    #[cfg(test)]
    #[pool_mutating_action("NoRequests")]
    #[pool_rollback]
    pub fn return_rollback_failure_chain(&mut self) -> StratisResult<()> {
        Err(StratisError::Chained(
            "Chained error".to_string(),
            Box::new(StratisError::RollbackError {
                causal_error: Box::new(StratisError::Msg("Causal error".to_string())),
                rollback_error: Box::new(StratisError::Msg("Rollback error".to_string())),
            }),
        ))
    }
}

impl<'a> Into<Value> for &'a StratPool {
    // Precondition: (&ThinPool).into() pattern matches Value::Object(_)
    // Precondition: (&Backstore).into() pattern matches Value::Object(_)
    fn into(self) -> Value {
        let mut map: Map<_, _> =
            if let Value::Object(map) = <&ThinPool as Into<Value>>::into(&self.thin_pool) {
                map.into_iter()
            } else {
                unreachable!("ThinPool conversion returns a JSON object")
            }
            .collect();
        map.extend(
            if let Value::Object(map) = <&Backstore as Into<Value>>::into(&self.backstore) {
                map.into_iter()
            } else {
                unreachable!("Backstore conversion returns a JSON object")
            },
        );
        map.insert(
            "available_actions".to_string(),
            Value::from(self.action_avail.to_string()),
        );
        Value::from(map)
    }
}

#[strat_pool_impl_gen]
impl Pool for StratPool {
    type Filesystem = StratFilesystem;
    type BlockDev = StratBlockDev;

    #[pool_mutating_action("NoRequests")]
    fn init_cache(
        &mut self,
        pool_uuid: PoolUuid,
        pool_name: &str,
        blockdevs: &[&Path],
    ) -> StratisResult<SetCreateAction<DevUuid>> {
        validate_paths(blockdevs)?;

        if self.is_encrypted() {
            return Err(StratisError::Msg(
                "Use of a cache is not supported with an encrypted pool".to_string(),
            ));
        }
        if !self.has_cache() {
            if blockdevs.is_empty() {
                return Err(StratisError::Msg(
                    "At least one blockdev path is required to initialize a cache.".to_string(),
                ));
            }

            // If adding cache devices, must suspend the pool, since the cache
            // must be augmented with the new devices.
            self.thin_pool.suspend()?;
            let devices_result = self.backstore.init_cache(pool_uuid, blockdevs);
            self.thin_pool.resume()?;
            let devices = devices_result?;
            self.write_metadata(pool_name)?;
            Ok(SetCreateAction::new(devices))
        } else {
            init_cache_idempotent_or_err(
                blockdevs,
                self.backstore
                    .cachedevs()
                    .into_iter()
                    .map(|(_, bd)| bd.physical_path().to_owned()),
            )
        }
    }

    #[pool_mutating_action("NoRequests")]
    #[pool_rollback]
    fn bind_clevis(
        &mut self,
        pin: &str,
        clevis_info: &Value,
    ) -> StratisResult<CreateAction<Clevis>> {
        let changed = self.backstore.bind_clevis(pin, clevis_info)?;
        if changed {
            Ok(CreateAction::Created(Clevis))
        } else {
            Ok(CreateAction::Identity)
        }
    }

    #[pool_mutating_action("NoRequests")]
    #[pool_rollback]
    fn unbind_clevis(&mut self) -> StratisResult<DeleteAction<Clevis>> {
        let changed = self.backstore.unbind_clevis()?;
        if changed {
            Ok(DeleteAction::Deleted(Clevis))
        } else {
            Ok(DeleteAction::Identity)
        }
    }

    #[pool_mutating_action("NoRequests")]
    #[pool_rollback]
    fn bind_keyring(
        &mut self,
        key_description: &KeyDescription,
    ) -> StratisResult<CreateAction<Key>> {
        let changed = self.backstore.bind_keyring(key_description)?;
        if changed {
            Ok(CreateAction::Created(Key))
        } else {
            Ok(CreateAction::Identity)
        }
    }

    #[pool_mutating_action("NoRequests")]
    #[pool_rollback]
    fn unbind_keyring(&mut self) -> StratisResult<DeleteAction<Key>> {
        let changed = self.backstore.unbind_keyring()?;
        if changed {
            Ok(DeleteAction::Deleted(Key))
        } else {
            Ok(DeleteAction::Identity)
        }
    }

    #[pool_mutating_action("NoRequests")]
    #[pool_rollback]
    fn rebind_keyring(
        &mut self,
        new_key_desc: &KeyDescription,
    ) -> StratisResult<RenameAction<Key>> {
        match self.backstore.rebind_keyring(new_key_desc)? {
            Some(true) => Ok(RenameAction::Renamed(Key)),
            Some(false) => Ok(RenameAction::Identity),
            None => Ok(RenameAction::NoSource),
        }
    }

    #[pool_mutating_action("NoRequests")]
    #[pool_rollback]
    fn rebind_clevis(&mut self) -> StratisResult<RegenAction> {
        self.backstore.rebind_clevis().map(|_| RegenAction)
    }

    #[pool_mutating_action("NoRequests")]
    fn create_filesystems<'a>(
        &mut self,
        pool_name: &str,
        pool_uuid: PoolUuid,
        specs: &[(&'a str, Option<Bytes>)],
    ) -> StratisResult<SetCreateAction<(&'a str, FilesystemUuid, Sectors)>> {
        let spec_map = validate_filesystem_size_specs(specs)?;

        spec_map.iter().fold(Ok(()), |res, (name, size)| {
            res.and_then(|()| validate_name(name))
                .and_then(|()| {
                    if let Some((_, fs)) = self.thin_pool.get_filesystem_by_name(name) {
                        if fs.thindev_size() == *size {
                            Ok(())
                        } else {
                            Err(StratisError::Msg(format!(
                                "Size {} of filesystem {} to be created conflicts with size {} for existing filesystem",
                                size,
                                name,
                                fs.thindev_size()
                            )))
                        }
                    } else {
                        Ok(())
                    }
                })
        })?;

        // TODO: Roll back on filesystem initialization failure.
        let mut result = Vec::new();
        for (name, size) in spec_map {
            if self.thin_pool.get_mut_filesystem_by_name(name).is_none() {
                let fs_uuid = self
                    .thin_pool
                    .create_filesystem(pool_name, pool_uuid, name, size)?;
                result.push((name, fs_uuid, size));
            }
        }

        Ok(SetCreateAction::new(result))
    }

    #[pool_mutating_action("NoRequests")]
    fn add_blockdevs(
        &mut self,
        pool_uuid: PoolUuid,
        pool_name: &str,
        paths: &[&Path],
        tier: BlockDevTier,
    ) -> StratisResult<SetCreateAction<DevUuid>> {
        validate_paths(paths)?;

        let bdev_info = if tier == BlockDevTier::Cache && !self.has_cache() {
            return Err(StratisError::Msg(
                format!(
                    "No cache has been initialized for pool with UUID {} and name {}; it is therefore impossible to add additional devices to the cache",
                    pool_uuid,
                    pool_name
                )
            ));
        } else if paths.is_empty() {
            //TODO: Substitute is_empty check with process_and_verify_devices
            return Ok(SetCreateAction::new(vec![]));
        } else if tier == BlockDevTier::Cache {
            // If adding cache devices, must suspend the pool; the cache
            // must be augmented with the new devices.
            self.thin_pool.suspend()?;
            let bdev_info_res = self
                .backstore
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

    #[pool_mutating_action("NoRequests")]
    fn destroy_filesystems(
        &mut self,
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

    #[pool_mutating_action("NoRequests")]
    fn rename_filesystem(
        &mut self,
        pool_name: &str,
        uuid: FilesystemUuid,
        new_name: &str,
    ) -> StratisResult<RenameAction<FilesystemUuid>> {
        validate_name(new_name)?;
        match self
            .thin_pool
            .rename_filesystem(pool_name, uuid, new_name)?
        {
            Some(true) => Ok(RenameAction::Renamed(uuid)),
            Some(false) => Ok(RenameAction::Identity),
            None => Ok(RenameAction::NoSource),
        }
    }

    #[pool_mutating_action("NoRequests")]
    fn snapshot_filesystem<'a>(
        &'a mut self,
        pool_name: &str,
        pool_uuid: PoolUuid,
        origin_uuid: FilesystemUuid,
        snapshot_name: &str,
    ) -> StratisResult<CreateAction<(FilesystemUuid, &'a mut Self::Filesystem)>> {
        validate_name(snapshot_name)?;

        if self
            .thin_pool
            .get_filesystem_by_name(snapshot_name)
            .is_some()
        {
            return Ok(CreateAction::Identity);
        }

        self.thin_pool
            .snapshot_filesystem(pool_name, pool_uuid, origin_uuid, snapshot_name)
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

    fn filesystems(&self) -> Vec<(Name, FilesystemUuid, &Self::Filesystem)> {
        self.thin_pool
            .filesystems()
            .into_iter()
            .map(|(n, u, f)| (n, u, f))
            .collect()
    }

    fn get_filesystem(&self, uuid: FilesystemUuid) -> Option<(Name, &Self::Filesystem)> {
        self.thin_pool
            .get_filesystem_by_uuid(uuid)
            .map(|(name, fs)| (name, fs))
    }

    fn get_filesystem_by_name(
        &self,
        fs_name: &Name,
    ) -> Option<(FilesystemUuid, &Self::Filesystem)> {
        self.thin_pool
            .get_filesystem_by_name(fs_name)
            .map(|(uuid, fs)| (uuid, fs))
    }

    fn blockdevs(&self) -> Vec<(DevUuid, BlockDevTier, &Self::BlockDev)> {
        self.backstore
            .blockdevs()
            .iter()
            .map(|&(u, t, b)| (u, t, b))
            .collect()
    }

    fn get_blockdev(&self, uuid: DevUuid) -> Option<(BlockDevTier, &Self::BlockDev)> {
        self.get_strat_blockdev(uuid).map(|(t, b)| (t, b))
    }

    #[pool_mutating_action("NoRequests")]
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

    fn has_cache(&self) -> bool {
        self.backstore.has_cache()
    }

    fn is_encrypted(&self) -> bool {
        self.datadevs_encrypted()
    }

    fn encryption_info(&self) -> Option<PoolEncryptionInfo> {
        self.backstore.data_tier_encryption_info()
    }

    fn avail_actions(&self) -> ActionAvailability {
        self.action_avail.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::OpenOptions,
        io::{BufWriter, Read, Write},
    };

    use nix::mount::{mount, umount, MsFlags};

    use devicemapper::{Bytes, ThinPoolStatus, ThinPoolStatusSummary, IEC, SECTOR_SIZE};

    use crate::engine::{
        engine::Filesystem,
        strat_engine::{
            cmd::udev_settle,
            tests::{loopbacked, real},
        },
        types::{EngineAction, Redundancy},
    };

    use super::*;

    fn invariant(pool: &StratPool, pool_name: &str) {
        check_metadata(&pool.record(&Name::new(pool_name.into()))).unwrap();
        assert!(!(pool.is_encrypted() && pool.backstore.has_cache()));
        if pool.avail_actions() == ActionAvailability::NoRequests {
            assert!(
                pool.encryption_info().is_some()
                    && pool
                        .encryption_info()
                        .map(|ei| { ei.is_inconsistent() })
                        .unwrap_or(false)
            );
        } else if pool.avail_actions() == ActionAvailability::Full {
            assert!(!pool
                .encryption_info()
                .map(|ei| ei.is_inconsistent())
                .unwrap_or(false));
        }
        assert!(pool
            .backstore
            .blockdevs()
            .iter()
            .all(|(_, _, bd)| bd.metadata_path().is_absolute()))
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
    fn loop_test_empty_pool() {
        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Exactly(0, None), test_empty_pool);
    }

    #[test]
    fn real_test_empty_pool() {
        real::test_with_spec(&real::DeviceLimits::Exactly(0, None, None), test_empty_pool);
    }

    /// Test that initializing a cache causes metadata to be updated. Verify
    /// that data written before the cache was initialized can be read
    /// afterwards.
    fn test_add_cachedevs(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let name = "stratis-test-pool";
        let (uuid, mut pool) = StratPool::initialize(name, paths2, Redundancy::NONE, None).unwrap();
        invariant(&pool, name);

        let metadata1 = pool.record(name);
        assert_matches!(metadata1.backstore.cache_tier, None);

        let (_, fs_uuid, _) = pool
            .create_filesystems(name, uuid, &[("stratis-filesystem", None)])
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
    }

    #[test]
    fn loop_test_add_cachedevs() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_add_cachedevs,
        );
    }

    #[test]
    fn real_test_add_cachedevs() {
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
        let (pool_uuid, mut pool) =
            StratPool::initialize(name, paths1, Redundancy::NONE, None).unwrap();
        invariant(&pool, name);

        let fs_name = "stratis_test_filesystem";
        let (_, fs_uuid, _) = pool
            .create_filesystems(name, pool_uuid, &[(fs_name, None)])
            .unwrap()
            .changed()
            .and_then(|mut fs| fs.pop())
            .expect("just created one");

        let devnode = pool.get_filesystem(fs_uuid).unwrap().1.devnode();

        {
            let buffer_length = IEC::Mi;
            let mut f = BufWriter::with_capacity(
                convert_test!(buffer_length, u64, usize),
                OpenOptions::new().write(true).open(devnode).unwrap(),
            );

            let buf = &[1u8; SECTOR_SIZE];

            let mut amount_written = Sectors(0);
            let buffer_length = Bytes::from(buffer_length).sectors();
            while match pool.thin_pool.state() {
                Some(ThinPoolStatus::Working(working)) => {
                    working.summary == ThinPoolStatusSummary::Good
                }
                _ => false,
            } {
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

            match pool.thin_pool.state() {
                Some(ThinPoolStatus::Working(working)) => {
                    assert_eq!(working.summary, ThinPoolStatusSummary::Good)
                }
                _ => panic!("thin pool status should be back to working"),
            }
        }
    }

    #[test]
    fn loop_test_add_datadevs() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, Some(Bytes::from(IEC::Gi * 4).sectors())),
            test_add_datadevs,
        );
    }

    #[test]
    fn real_test_add_datadevs() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(
                2,
                Some(Bytes::from(IEC::Gi * 2).sectors()),
                Some(Bytes::from(IEC::Gi * 4).sectors()),
            ),
            test_add_datadevs,
        );
    }

    /// Test that rollback errors are properly detected an maintenance mode
    /// is set accordingly.
    fn test_maintenance_mode(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let name = "stratis-test-pool";
        let (_, mut pool) = StratPool::initialize(name, paths, Redundancy::NONE, None).unwrap();
        invariant(&pool, name);

        assert_eq!(pool.action_avail, ActionAvailability::Full);
        assert!(pool.return_rollback_failure().is_err());
        assert_eq!(pool.action_avail, ActionAvailability::NoRequests);

        pool.destroy().unwrap();
        udev_settle().unwrap();

        let name = "stratis-test-pool";
        let (_, mut pool) = StratPool::initialize(name, paths, Redundancy::NONE, None).unwrap();
        invariant(&pool, name);

        assert_eq!(pool.action_avail, ActionAvailability::Full);
        assert!(pool.return_rollback_failure_chain().is_err());
        assert_eq!(pool.action_avail, ActionAvailability::NoRequests);
    }

    #[test]
    fn loop_test_maintenance_mode() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, Some(Bytes::from(IEC::Gi * 4).sectors())),
            test_maintenance_mode,
        );
    }

    #[test]
    fn real_test_mainenance_mode() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(
                2,
                Some(Bytes::from(IEC::Gi * 2).sectors()),
                Some(Bytes::from(IEC::Gi * 4).sectors()),
            ),
            test_maintenance_mode,
        );
    }
}

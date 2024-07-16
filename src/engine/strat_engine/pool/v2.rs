// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    vec::Vec,
};

use chrono::{DateTime, Utc};
use either::Either;
use serde_json::{Map, Value};

use devicemapper::{Bytes, DmNameBuf, Sectors};
use stratisd_proc_macros::strat_pool_impl_gen;

use crate::{
    engine::{
        engine::{BlockDev, DumpState, Filesystem, Pool, StateDiff},
        shared::{
            init_cache_idempotent_or_err, validate_filesystem_size, validate_filesystem_size_specs,
            validate_name, validate_paths,
        },
        strat_engine::{
            backstore::{
                backstore::{v2::Backstore, InternalBackstore},
                blockdev::{v2::StratBlockDev, InternalBlockDev},
                ProcessedPathInfos, UnownedDevices,
            },
            crypt::DEFAULT_CRYPT_DATA_OFFSET_V2,
            liminal::DeviceSet,
            metadata::{MDADataSize, BDA},
            serde_structs::{FlexDevsSave, PoolFeatures, PoolSave, Recordable},
            shared::tiers_to_bdas,
            thinpool::{StratFilesystem, ThinPool, ThinPoolSizeParams, DATA_BLOCK_SIZE},
            types::BDARecordResult,
        },
        types::{
            ActionAvailability, BlockDevTier, Clevis, Compare, CreateAction, DeleteAction, DevUuid,
            Diff, EncryptedDevice, EncryptionInfo, FilesystemUuid, GrowAction, InputEncryptionInfo,
            Key, KeyDescription, Name, OffsetDirection, OptionalTokenSlotInput, PoolDiff,
            PoolEncryptionInfo, PoolUuid, PropChangeAction, RegenAction, RenameAction,
            SetCreateAction, SetDeleteAction, SizedKeyMemory, StratFilesystemDiff, StratPoolDiff,
            StratSigblockVersion, TokenUnlockMethod, ValidatedIntegritySpec,
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
    [
        &flex_devs.meta_dev,
        &flex_devs.thin_meta_dev,
        &flex_devs.thin_data_dev,
        &flex_devs.thin_meta_dev_spare,
    ]
    .iter()
    .flat_map(|vec| vec.iter().map(|(_, length)| *length))
    .sum()
}

/// Check the metadata of an individual pool for consistency.
/// Precondition: This method is called only when setting up a pool, which
/// ensures that the flex devs metadata lists are all non-empty.
fn check_metadata(metadata: &PoolSave) -> StratisResult<()> {
    let flex_devs = &metadata.flex_devs;
    let next = next_index(flex_devs);
    let allocated_from_cap = metadata
        .backstore
        .cap
        .allocs
        .iter()
        .map(|(_, size)| *size)
        .sum::<Sectors>();

    if allocated_from_cap != next {
        let err_msg = format!(
            "{next} used in thinpool, but {allocated_from_cap} allocated from backstore cap device"
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
                "{next} allocated to cap device, but {total_allocated} allocated to flex devs"
            );
            return Err(StratisError::Msg(err_msg));
        }
    }

    Ok(())
}

#[derive(Debug)]
pub struct StratPool {
    backstore: Backstore,
    thin_pool: ThinPool<Backstore>,
    action_avail: ActionAvailability,
    metadata_size: Sectors,
}

#[strat_pool_impl_gen]
impl StratPool {
    /// Initialize a Stratis Pool.
    /// 1. Initialize the block devices specified by paths.
    /// 2. Set up thinpool device to back filesystems.
    /// Precondition: p.is_absolute() is true for all p in paths
    pub fn initialize(
        name: &str,
        devices: UnownedDevices,
        encryption_info: Option<&InputEncryptionInfo>,
        integrity_spec: ValidatedIntegritySpec,
    ) -> StratisResult<(PoolUuid, StratPool)> {
        let pool_uuid = PoolUuid::new_v4();

        // FIXME: Initializing with the minimum MDA size is not necessarily
        // enough. If there are enough devices specified, more space will be
        // required.
        let mut backstore = Backstore::initialize(
            pool_uuid,
            devices,
            MDADataSize::default(),
            encryption_info,
            integrity_spec,
        )?;

        let thinpool = ThinPool::<Backstore>::new(
            pool_uuid,
            match ThinPoolSizeParams::new(backstore.available_in_backstore()) {
                Ok(ref params) => params,
                Err(causal_error) => {
                    if let Err(cleanup_err) = backstore.destroy(pool_uuid) {
                        warn!("Failed to clean up Stratis metadata for incompletely set up pool with UUID {}: {}.", pool_uuid, cleanup_err);
                        return Err(StratisError::NoActionRollbackError {
                            causal_error: Box::new(causal_error),
                            rollback_error: Box::new(cleanup_err),
                        });
                    }
                    return Err(causal_error);
                }
            },
            DATA_BLOCK_SIZE,
            &mut backstore,
        );

        let thinpool = match thinpool {
            Ok(thinpool) => thinpool,
            Err(causal_error) => {
                if let Err(cleanup_err) = backstore.destroy(pool_uuid) {
                    warn!("Failed to clean up Stratis metadata for incompletely set up pool with UUID {}: {}.", pool_uuid, cleanup_err);
                    return Err(StratisError::NoActionRollbackError {
                        causal_error: Box::new(causal_error),
                        rollback_error: Box::new(cleanup_err),
                    });
                }
                return Err(causal_error);
            }
        };

        let metadata_size = backstore.datatier_metadata_size();
        let mut pool = StratPool {
            backstore,
            thin_pool: thinpool,
            action_avail: ActionAvailability::Full,
            metadata_size,
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
        token_slot: TokenUnlockMethod,
        passphrase: Option<SizedKeyMemory>,
    ) -> BDARecordResult<(Name, StratPool)> {
        if let Err(e) = check_metadata(metadata) {
            return Err((e, tiers_to_bdas(datadevs, cachedevs, None)));
        }

        let backstore = Backstore::setup(
            uuid, metadata, datadevs, cachedevs, timestamp, token_slot, passphrase,
        )?;
        let action_avail = backstore.action_availability();

        let pool_name = &metadata.name;

        if action_avail != ActionAvailability::Full {
            warn!(
                "Disabling some actions for pool {} with UUID {}; pool is designated {}",
                pool_name, uuid, action_avail
            );
        }

        let thinpool = match ThinPool::setup(
            pool_name,
            uuid,
            &metadata.thinpool_dev,
            &metadata.flex_devs,
            &backstore,
        ) {
            Ok(tp) => tp,
            Err(e) => return Err((e, backstore.into_bdas())),
        };

        // TODO: Remove in stratisd 4.0
        let mut needs_save = metadata.thinpool_dev.fs_limit.is_none()
            || metadata.thinpool_dev.feature_args.is_none();

        let metadata_size = backstore.datatier_metadata_size();
        let mut pool = StratPool {
            backstore,
            thin_pool: thinpool,
            action_avail,
            metadata_size,
        };

        // The value of the started field in the pool metadata needs to be
        // updated unless the value is already present in the metadata and has
        // value true.
        needs_save |= !metadata.started.unwrap_or(false);

        if needs_save {
            if let Err(err) = pool.write_metadata(pool_name) {
                if let StratisError::ActionDisabled(avail) = err {
                    warn!("Pool-level metadata could not be written for pool with name {} and UUID {} because pool is in a limited availability state, {},  which prevents any pool actions; pool will remain set up", pool_name, uuid, avail);
                } else {
                    return Err((err, pool.backstore.into_bdas()));
                }
            }
        }

        Ok((Name::new(pool_name.to_owned()), pool))
    }

    fn get_filesystem(&self, uuid: FilesystemUuid) -> Option<(Name, &StratFilesystem)> {
        self.thin_pool.get_filesystem_by_uuid(uuid)
    }

    fn get_filesystem_by_name(&self, fs_name: &Name) -> Option<(FilesystemUuid, &StratFilesystem)> {
        self.thin_pool.get_filesystem_by_name(fs_name)
    }

    /// Send a synthetic udev change event to every filesystem on the given pool.
    pub fn udev_pool_change(&self, pool_name: &str) {
        for (name, uuid, fs) in self.thin_pool.filesystems() {
            fs.udev_fs_change(pool_name, uuid, &name);
        }
    }

    /// Write current metadata to pool members.
    #[pool_mutating_action("NoPoolChanges")]
    pub fn write_metadata(&mut self, name: &str) -> StratisResult<()> {
        let data = serde_json::to_string(&self.record(name))?;
        self.backstore.save_state(data.as_bytes())
    }

    /// Teardown a pool.
    #[cfg(test)]
    pub fn teardown(&mut self, pool_uuid: PoolUuid) -> StratisResult<()> {
        self.thin_pool.teardown(pool_uuid).map_err(|(e, _)| e)?;
        self.backstore.teardown(pool_uuid)?;
        Ok(())
    }

    pub fn has_filesystems(&self) -> bool {
        self.thin_pool.has_filesystems()
    }

    /// The names of DM devices belonging to this pool that may generate events
    pub fn get_eventing_dev_names(&self, pool_uuid: PoolUuid) -> Vec<DmNameBuf> {
        self.thin_pool.get_eventing_dev_names(pool_uuid)
    }

    /// Called when a DM device in this pool has generated an event. This method
    /// handles checking pools.
    #[pool_mutating_action("NoPoolChanges")]
    pub fn event_on(&mut self, pool_uuid: PoolUuid, pool_name: &Name) -> StratisResult<PoolDiff> {
        let cached = self.cached();
        let (changed, thin_pool) = self.thin_pool.check(pool_uuid, &mut self.backstore)?;
        let pool = cached.diff(&self.dump(()));
        if changed {
            self.write_metadata(pool_name)?;
        }
        Ok(PoolDiff { thin_pool, pool })
    }

    /// Called when a DM device in this pool has generated an event. This method
    /// handles checking filesystems.
    #[pool_mutating_action("NoPoolChanges")]
    pub fn fs_event_on(
        &mut self,
        pool_uuid: PoolUuid,
    ) -> StratisResult<HashMap<FilesystemUuid, StratFilesystemDiff>> {
        self.thin_pool.check_fs(pool_uuid, &self.backstore)
    }

    pub fn record(&self, name: &str) -> PoolSave {
        let mut features = vec![];
        if self.is_encrypted() {
            features.push(PoolFeatures::Encryption);
        }
        PoolSave {
            name: name.to_owned(),
            backstore: self.backstore.record(),
            flex_devs: self.thin_pool.record(),
            thinpool_dev: self.thin_pool.record(),
            started: Some(true),
            features,
        }
    }

    pub fn get_strat_blockdev(&self, uuid: DevUuid) -> Option<(BlockDevTier, &StratBlockDev)> {
        self.backstore.get_blockdev_by_uuid(uuid)
    }

    #[pool_mutating_action("NoPoolChanges")]
    pub fn get_mut_strat_blockdev(
        &mut self,
        uuid: DevUuid,
    ) -> StratisResult<Option<(BlockDevTier, &mut StratBlockDev)>> {
        Ok(self.backstore.get_mut_blockdev_by_uuid(uuid))
    }

    pub fn blockdevs(&self) -> Vec<(DevUuid, BlockDevTier, &StratBlockDev)> {
        self.backstore.blockdevs()
    }

    #[pool_mutating_action("NoPoolChanges")]
    pub fn blockdevs_mut(
        &mut self,
    ) -> StratisResult<Vec<(DevUuid, BlockDevTier, &mut StratBlockDev)>> {
        Ok(self.backstore.blockdevs_mut())
    }

    /// Destroy the pool.
    /// Precondition: All filesystems belonging to this pool must be
    /// unmounted.
    ///
    /// This method is not a mutating action as the pool should be allowed
    /// to be destroyed even if the metadata is inconsistent.
    pub fn destroy(&mut self, pool_uuid: PoolUuid) -> Result<(), (StratisError, bool)> {
        self.thin_pool.teardown(pool_uuid)?;
        self.backstore.destroy(pool_uuid).map_err(|e| (e, false))?;
        Ok(())
    }

    /// Check the limit of filesystems on a pool and return an error if it has been passed.
    fn check_fs_limit(&self, new_fs: usize) -> StratisResult<()> {
        let fs_limit = self.fs_limit();
        if convert_int!(fs_limit, u64, usize)? < self.filesystems().len() + new_fs {
            Err(StratisError::Msg(format!("The pool limit of {fs_limit} filesystems has already been reached; increase the filesystem limit on the pool to continue")))
        } else {
            Ok(())
        }
    }

    /// Stop a pool, consuming it and converting it into a set of devices to be
    /// set up again later.
    pub fn stop(
        &mut self,
        pool_name: &Name,
        pool_uuid: PoolUuid,
    ) -> Result<DeviceSet, (StratisError, bool)> {
        self.thin_pool.teardown(pool_uuid)?;
        let mut data = self.record(pool_name);
        data.started = Some(false);
        let json = serde_json::to_string(&data).map_err(|e| (StratisError::from(e), false))?;
        self.backstore
            .save_state(json.as_bytes())
            .map_err(|e| (e, false))?;
        self.backstore.teardown(pool_uuid).map_err(|e| (e, false))?;
        let bds = self.backstore.drain_bds();
        Ok(DeviceSet::from(bds))
    }

    /// Convert a pool into a record of BDAs for the given block devices in the pool.
    pub fn into_bdas(self) -> HashMap<DevUuid, BDA> {
        self.backstore.into_bdas()
    }

    /// Drain pool block devices into a record of block devices in the pool.
    pub fn drain_bds(&mut self) -> Vec<StratBlockDev> {
        self.backstore.drain_bds()
    }

    #[cfg(test)]
    #[pool_mutating_action("NoRequests")]
    #[pool_rollback]
    pub fn return_rollback_failure(&mut self) -> StratisResult<()> {
        Err(StratisError::RollbackError {
            causal_error: Box::new(StratisError::Msg("Causal error".to_string())),
            rollback_error: Box::new(StratisError::Msg("Rollback error".to_string())),
            level: ActionAvailability::NoRequests,
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
                level: ActionAvailability::NoRequests,
            }),
        ))
    }

    /// Verifies that the filesystem operation to be performed is allowed to perform
    /// overprovisioning if it is determined to be the end result.
    fn check_overprov(&self, increase: Sectors) -> StratisResult<()> {
        let cur_filesystem_size_sum = self.thin_pool.filesystem_logical_size_sum()?;
        if !self.thin_pool.overprov_enabled()
            && cur_filesystem_size_sum + increase > self.thin_pool.total_fs_limit(&self.backstore)
        {
            Err(StratisError::Msg(format!(
                "Overprovisioning is disabled on this pool and increasing total filesystem size ({cur_filesystem_size_sum}) by {increase} would result in overprovisioning"
            )))
        } else {
            Ok(())
        }
    }
}

impl Into<Value> for &StratPool {
    // Precondition: (&ThinPool).into() pattern matches Value::Object(_)
    // Precondition: (&Backstore).into() pattern matches Value::Object(_)
    fn into(self) -> Value {
        let mut map: Map<_, _> = if let Value::Object(map) =
            <&ThinPool<Backstore> as Into<Value>>::into(&self.thin_pool)
        {
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
        map.insert("fs_limit".to_string(), Value::from(self.fs_limit()));
        Value::from(map)
    }
}

#[strat_pool_impl_gen]
impl Pool for StratPool {
    #[pool_mutating_action("NoRequests")]
    fn init_cache(
        &mut self,
        pool_uuid: PoolUuid,
        pool_name: &str,
        blockdevs: &[&Path],
        supports_encrypted: bool,
    ) -> StratisResult<SetCreateAction<DevUuid>> {
        validate_paths(blockdevs)?;

        if self.is_encrypted() && !supports_encrypted {
            return Err(StratisError::Msg(
                "Use of a cache is not supported with an encrypted pool".to_string(),
            ));
        }

        let devices = ProcessedPathInfos::try_from(blockdevs)?;
        let (stratis_devices, unowned_devices) = devices.unpack();
        let (this_pool, other_pools) = stratis_devices.partition(pool_uuid);

        other_pools.error_on_not_empty()?;

        let (in_pool, out_pool): (Vec<_>, Vec<_>) = this_pool
            .keys()
            .map(|dev_uuid| {
                self.backstore
                    .get_blockdev_by_uuid(*dev_uuid)
                    .map(|(tier, _)| (*dev_uuid, tier))
            })
            .partition(|v| v.is_some());

        if !out_pool.is_empty() {
            let error_message = format!(
                    "Devices ({}) appear to be already in use by this pool which has UUID {} but this pool has no record of them",
                    out_pool
                    .iter()
                    .map(|opt| this_pool.get(&opt.expect("was looked up").0).expect("partitioned from this_pool").devnode.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                    pool_uuid
                );
            return Err(StratisError::Msg(error_message));
        };

        let (datadevs, cachedevs): (Vec<_>, Vec<_>) = in_pool
            .iter()
            .map(|opt| opt.expect("in_pool devices are Some"))
            .partition(|(_, tier)| *tier == BlockDevTier::Data);

        if !datadevs.is_empty() {
            let error_message = format!(
                "Devices ({}) appear to be already in use by this pool which has UUID {} in the data tier",
                datadevs
                    .iter()
                    .map(|(uuid, _)| this_pool.get(uuid).expect("partitioned from this_pool").devnode.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                pool_uuid
            );
            return Err(StratisError::Msg(error_message));
        };

        if !self.has_cache() {
            if unowned_devices.is_empty() {
                return Err(StratisError::Msg(
                    "At least one device is required to initialize a cache.".to_string(),
                ));
            }

            let block_size_summary = unowned_devices.blocksizes();
            if block_size_summary.len() > 1 {
                let err_str = "The devices specified for the cache tier do not all have the same physical sector size or do not all have the same logical sector size.".into();
                return Err(StratisError::Msg(err_str));
            }

            let cache_sector_sizes = block_size_summary
                .keys()
                .next()
                .expect("unowned_devices is not empty");

            let current_data_sector_sizes = self
                .backstore
                .block_size_summary(BlockDevTier::Data)
                .expect("always exists for data tier")
                .validate()
                .expect("All operations prevented if validate() function returns an error");

            if cache_sector_sizes.logical_sector_size
                != current_data_sector_sizes.base.logical_sector_size
            {
                let err_str = "The logical sector size of the devices proposed for the cache tier does not match the effective logical sector size of the data tier".to_string();
                return Err(StratisError::Msg(err_str));
            }

            self.thin_pool.suspend()?;
            let devices_result = self.backstore.init_cache(pool_uuid, unowned_devices);
            self.thin_pool.resume()?;
            let devices = devices_result?;
            self.write_metadata(pool_name)?;
            Ok(SetCreateAction::new(devices))
        } else {
            init_cache_idempotent_or_err(
                &cachedevs
                    .iter()
                    .map(|(uuid, _)| {
                        this_pool
                            .get(uuid)
                            .expect("partitioned from this_pool")
                            .devnode
                            .as_path()
                    })
                    .chain(
                        unowned_devices
                            .unpack()
                            .iter()
                            .map(|info| info.devnode.as_path()),
                    )
                    .collect::<Vec<_>>(),
                self.backstore
                    .cachedevs()
                    .into_iter()
                    .map(|(_, bd)| bd.physical_path().to_owned()),
            )
        }
    }

    #[pool_mutating_action("NoRequests")]
    fn bind_clevis(
        &mut self,
        token_slot: OptionalTokenSlotInput,
        pin: &str,
        clevis_info: &Value,
    ) -> StratisResult<CreateAction<(Clevis, u32)>> {
        let changed = self.backstore.bind_clevis(token_slot, pin, clevis_info)?;
        match changed {
            Some(t) => Ok(CreateAction::Created((Clevis, t))),
            None => Ok(CreateAction::Identity),
        }
    }

    #[pool_mutating_action("NoRequests")]
    fn bind_keyring(
        &mut self,
        token_slot: OptionalTokenSlotInput,
        key_description: &KeyDescription,
    ) -> StratisResult<CreateAction<(Key, u32)>> {
        let changed = self.backstore.bind_keyring(token_slot, key_description)?;
        match changed {
            Some(t) => Ok(CreateAction::Created((Key, t))),
            None => Ok(CreateAction::Identity),
        }
    }

    #[pool_mutating_action("NoRequests")]
    fn rebind_keyring(
        &mut self,
        token_slot: Option<u32>,
        new_key_desc: &KeyDescription,
    ) -> StratisResult<RenameAction<Key>> {
        match self.backstore.rebind_keyring(token_slot, new_key_desc)? {
            Some(true) => Ok(RenameAction::Renamed(Key)),
            Some(false) => Ok(RenameAction::Identity),
            None => Ok(RenameAction::NoSource),
        }
    }

    #[pool_mutating_action("NoRequests")]
    fn rebind_clevis(&mut self, token_slot: Option<u32>) -> StratisResult<RegenAction> {
        self.backstore
            .rebind_clevis(token_slot)
            .map(|_| RegenAction)
    }

    #[pool_mutating_action("NoRequests")]
    fn unbind_keyring(&mut self, token_slot: Option<u32>) -> StratisResult<DeleteAction<Key>> {
        let changed = self.backstore.unbind_keyring(token_slot)?;
        if changed {
            Ok(DeleteAction::Deleted(Key))
        } else {
            Ok(DeleteAction::Identity)
        }
    }

    #[pool_mutating_action("NoRequests")]
    fn unbind_clevis(&mut self, token_slot: Option<u32>) -> StratisResult<DeleteAction<Clevis>> {
        let changed = self.backstore.unbind_clevis(token_slot)?;
        if changed {
            Ok(DeleteAction::Deleted(Clevis))
        } else {
            Ok(DeleteAction::Identity)
        }
    }

    #[pool_mutating_action("NoRequests")]
    fn create_filesystems<'a>(
        &mut self,
        pool_name: &str,
        pool_uuid: PoolUuid,
        specs: &[(&'a str, Option<Bytes>, Option<Bytes>)],
    ) -> StratisResult<SetCreateAction<(&'a str, FilesystemUuid, Sectors)>> {
        self.check_fs_limit(specs.len())?;

        let spec_map = validate_filesystem_size_specs(specs)?;

        let increase = spec_map
            .values()
            .map(|(size, _)| size)
            .copied()
            .sum::<Sectors>();
        self.check_overprov(increase)?;

        spec_map.iter().try_fold((), |_, (name, (size, _))| {
            validate_name(name)
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
        for (name, (size, size_limit)) in spec_map {
            if self.thin_pool.get_mut_filesystem_by_name(name).is_none() {
                let fs_uuid = self
                    .thin_pool
                    .create_filesystem(pool_name, pool_uuid, name, size, size_limit)?;
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
    ) -> StratisResult<(SetCreateAction<DevUuid>, Option<PoolDiff>)> {
        validate_paths(paths)?;

        let bdev_info = if tier == BlockDevTier::Cache && !self.has_cache() {
            return Err(StratisError::Msg(
                format!(
                    "No cache has been initialized for pool with UUID {pool_uuid} and name {pool_name}; it is therefore impossible to add additional devices to the cache"
                )
            ));
        } else {
            let devices = ProcessedPathInfos::try_from(paths)?;
            let (stratis_devices, unowned_devices) = devices.unpack();
            let (this_pool, other_pools) = stratis_devices.partition(pool_uuid);

            other_pools.error_on_not_empty()?;

            let (in_pool, out_pool): (Vec<_>, Vec<_>) = this_pool
                .keys()
                .map(|dev_uuid| {
                    self.backstore
                        .get_blockdev_by_uuid(*dev_uuid)
                        .map(|(tier, _)| (*dev_uuid, tier))
                })
                .partition(|v| v.is_some());

            if !out_pool.is_empty() {
                let error_message = format!(
                    "Devices ({}) appear to be already in use by this pool which has UUID {} but this pool has no record of them",
                    out_pool
                    .iter()
                    .map(|opt| this_pool.get(&opt.expect("was looked up").0).expect("partitioned from this_pool").devnode.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                    pool_uuid
                );
                return Err(StratisError::Msg(error_message));
            };

            let (datadevs, cachedevs): (Vec<_>, Vec<_>) = in_pool
                .iter()
                .map(|opt| opt.expect("in_pool devices are Some"))
                .partition(|(_, tier)| *tier == BlockDevTier::Data);

            if tier == BlockDevTier::Cache {
                // If adding cache devices, must suspend the pool; the cache
                // must be augmented with the new devices.
                if !datadevs.is_empty() {
                    let error_message = format!(
                        "Devices ({}) appear to be already in use by this pool which has UUID {}, but in the data tier not the cache tier",
                        datadevs
                        .iter()
                        .map(|(uuid, _)| this_pool.get(uuid).expect("partitioned from this_pool").devnode.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                        pool_uuid
                    );
                    return Err(StratisError::Msg(error_message));
                };

                if unowned_devices.is_empty() {
                    return Ok((SetCreateAction::new(vec![]), None));
                }

                let block_size_summary = unowned_devices.blocksizes();
                if block_size_summary.len() > 1 {
                    let err_str = "The devices specified to be added to the cache tier do not all have the same physical sector size or do not all have the same logical sector size.".into();
                    return Err(StratisError::Msg(err_str));
                }
                let added_sector_sizes = block_size_summary
                    .keys()
                    .next()
                    .expect("unowned devices is not empty");

                let current_sector_sizes = self
                    .backstore
                    .block_size_summary(BlockDevTier::Cache)
                    .expect("already returned if no cache tier")
                    .validate()
                    .expect("All devices of the cache tier must be in use, so there can only be one representative logical sector size.");

                if !(&current_sector_sizes.base == added_sector_sizes) {
                    let err_str = format!("The sector sizes of the devices proposed for extending the cache tier, {added_sector_sizes}, do not match the effective sector sizes of the existing cache devices, {0}", current_sector_sizes.base);
                    return Err(StratisError::Msg(err_str));
                }

                self.thin_pool.suspend()?;
                let bdev_info_res = self.backstore.add_cachedevs(pool_uuid, unowned_devices);
                self.thin_pool.resume()?;
                let bdev_info = bdev_info_res?;
                Ok((SetCreateAction::new(bdev_info), None))
            } else {
                if !cachedevs.is_empty() {
                    let error_message = format!(
                        "Devices ({}) appear to be already in use by this pool which has UUID {}, but in the cache tier not the data tier",
                        cachedevs
                        .iter()
                        .map(|(uuid, _)| this_pool.get(uuid).expect("partitioned from this_pool").devnode.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                        pool_uuid
                    );
                    return Err(StratisError::Msg(error_message));
                };

                if unowned_devices.is_empty() {
                    return Ok((SetCreateAction::new(vec![]), None));
                }

                let block_size_summary = unowned_devices.blocksizes();
                if block_size_summary.len() > 1 {
                    let err_str = "The devices specified to be added to the data tier do not have uniform physical and logical sector sizes.".into();
                    return Err(StratisError::Msg(err_str));
                }

                let added_sector_sizes = block_size_summary
                    .keys()
                    .next()
                    .expect("unowned devices is not empty");

                let current_sector_sizes = self
                    .backstore
                    .block_size_summary(BlockDevTier::Data)
                    .expect("always exists")
                    .validate()
                    .expect("All operations prevented if validate() function on data tier block size summary returns an error");

                if !(&current_sector_sizes.base == added_sector_sizes) {
                    let err_str = format!("The sector sizes of the devices proposed for extending the data tier, {added_sector_sizes}, do not match the effective sector sizes of the existing data devices, {0}", current_sector_sizes.base);
                    return Err(StratisError::Msg(err_str));
                }

                let cached = self.cached();

                // If just adding data devices, no need to suspend the pool.
                // No action will be taken on the DM devices.
                let bdev_info = self.backstore.add_datadevs(pool_uuid, unowned_devices)?;
                self.thin_pool.set_queue_mode();
                self.thin_pool.clear_out_of_meta_flag();

                Ok((
                    SetCreateAction::new(bdev_info),
                    Some(PoolDiff {
                        thin_pool: self.thin_pool.cached().unchanged(),
                        pool: cached.diff(&self.dump(())),
                    }),
                ))
            }
        };
        self.write_metadata(pool_name)?;
        bdev_info
    }

    #[pool_mutating_action("NoRequests")]
    fn destroy_filesystems(
        &mut self,
        pool_name: &str,
        fs_uuids: &HashSet<FilesystemUuid>,
    ) -> StratisResult<SetDeleteAction<FilesystemUuid, (FilesystemUuid, Option<FilesystemUuid>)>>
    {
        self.thin_pool.destroy_filesystems(pool_name, fs_uuids)
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
    ) -> StratisResult<CreateAction<(FilesystemUuid, &'a mut dyn Filesystem)>> {
        self.check_fs_limit(1)?;

        validate_name(snapshot_name)?;
        self.check_overprov(
            self.thin_pool
                .get_filesystem_by_uuid(origin_uuid)
                .ok_or_else(|| {
                    StratisError::Msg(format!(
                        "Filesystem with UUID {origin_uuid} could not be found"
                    ))
                })?
                .1
                .thindev_size(),
        )?;

        if self
            .thin_pool
            .get_filesystem_by_name(snapshot_name)
            .is_some()
        {
            return Ok(CreateAction::Identity);
        }

        self.thin_pool
            .snapshot_filesystem(pool_name, pool_uuid, origin_uuid, snapshot_name)
            .map(|(uuid, fs)| CreateAction::Created((uuid, fs as &mut dyn Filesystem)))
    }

    fn total_physical_size(&self) -> Sectors {
        self.backstore.datatier_size()
    }

    fn total_allocated_size(&self) -> Sectors {
        self.backstore.datatier_allocated_size() + self.metadata_size
    }

    fn total_physical_used(&self) -> Option<Sectors> {
        // TODO: note that with the addition of another layer, the
        // calculation of the amount of physical spaced used by means
        // of adding the amount used by Stratis metadata to the amount used
        // by the pool abstraction will be invalid. In the event of, e.g.,
        // software RAID, the amount will be far too low to be useful, in the
        // event of, e.g, VDO, the amount will be far too large to be useful.
        self.thin_pool
            .total_physical_used()
            .map(|u| u + self.metadata_size)
    }

    fn filesystems(&self) -> Vec<(Name, FilesystemUuid, &dyn Filesystem)> {
        self.thin_pool
            .filesystems()
            .into_iter()
            .map(|(n, u, f)| (n, u, f as &dyn Filesystem))
            .collect()
    }

    fn get_filesystem(&self, uuid: FilesystemUuid) -> Option<(Name, &dyn Filesystem)> {
        self.get_filesystem(uuid)
            .map(|(name, fs)| (name, fs as &dyn Filesystem))
    }

    fn get_filesystem_by_name(&self, fs_name: &Name) -> Option<(FilesystemUuid, &dyn Filesystem)> {
        self.get_filesystem_by_name(fs_name)
            .map(|(uuid, fs)| (uuid, fs as &dyn Filesystem))
    }

    fn blockdevs(&self) -> Vec<(DevUuid, BlockDevTier, &dyn BlockDev)> {
        self.backstore
            .blockdevs()
            .into_iter()
            .map(|(uuid, tier, bd)| (uuid, tier, bd as &dyn BlockDev))
            .collect::<Vec<_>>()
    }

    fn get_blockdev(&self, uuid: DevUuid) -> Option<(BlockDevTier, &dyn BlockDev)> {
        self.get_strat_blockdev(uuid)
            .map(|(t, bd)| (t, bd as &dyn BlockDev))
    }

    #[pool_mutating_action("NoRequests")]
    fn get_mut_blockdev(
        &mut self,
        uuid: DevUuid,
    ) -> StratisResult<Option<(BlockDevTier, &mut dyn BlockDev)>> {
        self.get_mut_strat_blockdev(uuid)
            .map(|opt| opt.map(|(t, bd)| (t, bd as &mut dyn BlockDev)))
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
        self.backstore.is_encrypted()
    }

    fn encryption_info_legacy(&self) -> Option<PoolEncryptionInfo> {
        self.backstore
            .encryption_info()
            .map(PoolEncryptionInfo::from)
    }

    fn encryption_info(&self) -> Option<Either<EncryptionInfo, PoolEncryptionInfo>> {
        self.backstore.encryption_info().cloned().map(Either::Left)
    }

    fn avail_actions(&self) -> ActionAvailability {
        self.action_avail.clone()
    }

    fn fs_limit(&self) -> u64 {
        self.thin_pool.fs_limit()
    }

    #[pool_mutating_action("NoPoolChanges")]
    fn set_fs_limit(
        &mut self,
        pool_name: &Name,
        pool_uuid: PoolUuid,
        new_limit: u64,
    ) -> StratisResult<()> {
        let (should_save, res) =
            self.thin_pool
                .set_fs_limit(pool_uuid, &mut self.backstore, new_limit);
        if should_save {
            self.write_metadata(pool_name)?;
        }
        res
    }

    fn overprov_enabled(&self) -> bool {
        self.thin_pool.overprov_enabled()
    }

    #[pool_mutating_action("NoPoolChanges")]
    fn set_overprov_mode(&mut self, pool_name: &Name, enabled: bool) -> StratisResult<()> {
        let (should_save, res) = self.thin_pool.set_overprov_mode(&self.backstore, enabled);
        if should_save {
            self.write_metadata(pool_name)?;
        }
        res
    }

    fn out_of_alloc_space(&self) -> bool {
        self.thin_pool.out_of_alloc_space()
    }

    #[pool_mutating_action("NoRequests")]
    fn grow_physical(
        &mut self,
        name: &Name,
        pool_uuid: PoolUuid,
        device: DevUuid,
    ) -> StratisResult<(GrowAction<(PoolUuid, DevUuid)>, Option<PoolDiff>)> {
        let cached = self.cached();

        let changed = self.backstore.grow(device)?;
        if changed {
            if self.thin_pool.set_queue_mode() {
                self.write_metadata(name)?;
            }
            Ok((
                GrowAction::Grown((pool_uuid, device)),
                Some(PoolDiff {
                    thin_pool: self.thin_pool.cached().unchanged(),
                    pool: cached.diff(&self.dump(())),
                }),
            ))
        } else {
            Ok((GrowAction::Identity, None))
        }
    }

    #[pool_mutating_action("NoRequests")]
    fn set_fs_size_limit(
        &mut self,
        fs_uuid: FilesystemUuid,
        limit: Option<Bytes>,
    ) -> StratisResult<PropChangeAction<Option<Sectors>>> {
        let (name, _) = self.get_filesystem(fs_uuid).ok_or_else(|| {
            StratisError::Msg(format!("Filesystem with UUID {fs_uuid} not found"))
        })?;
        let limit = validate_filesystem_size(&name, limit)?;
        if self.thin_pool.set_fs_size_limit(fs_uuid, limit)? {
            Ok(PropChangeAction::NewValue(limit))
        } else {
            Ok(PropChangeAction::Identity)
        }
    }

    #[pool_mutating_action("NoRequests")]
    fn encrypt_pool(
        &mut self,
        name: &Name,
        pool_uuid: PoolUuid,
        encryption_info: &InputEncryptionInfo,
    ) -> StratisResult<CreateAction<EncryptedDevice>> {
        match self.backstore.encryption_info() {
            Some(_) => Ok(CreateAction::Identity),
            None => {
                self.thin_pool.suspend()?;
                let encrypt_res = self
                    .backstore
                    .encrypt(pool_uuid, &mut self.thin_pool, encryption_info)
                    .map(|_| {
                        self.thin_pool.set_device(
                            self.backstore.device().expect(
                                "Since thin pool exists, space must have been allocated \
                             from the backstore, so backstore must have a cap device",
                            ),
                            DEFAULT_CRYPT_DATA_OFFSET_V2,
                            OffsetDirection::Backwards,
                        )
                    });
                self.thin_pool.resume()?;
                let metadata_res = self.write_metadata(name);
                let _ = encrypt_res?;
                metadata_res?;
                Ok(CreateAction::Created(EncryptedDevice))
            }
        }
    }

    fn current_metadata(&self, pool_name: &Name) -> StratisResult<String> {
        serde_json::to_string(&self.record(pool_name)).map_err(|e| e.into())
    }

    fn last_metadata(&self) -> StratisResult<String> {
        self.backstore.load_state().and_then(|v| {
            String::from_utf8(v)
                .map_err(|_| StratisError::Msg("metadata byte array is not utf-8".into()))
        })
    }

    fn metadata_version(&self) -> StratSigblockVersion {
        StratSigblockVersion::V2
    }

    fn current_fs_metadata(&self, fs_name: Option<&str>) -> StratisResult<String> {
        self.thin_pool.current_fs_metadata(fs_name)
    }

    fn last_fs_metadata(&self, fs_name: Option<&str>) -> StratisResult<String> {
        self.thin_pool.last_fs_metadata(fs_name)
    }

    #[pool_mutating_action("NoRequests")]
    fn set_fs_merge_scheduled(
        &mut self,
        fs_uuid: FilesystemUuid,
        new_scheduled: bool,
    ) -> StratisResult<PropChangeAction<bool>> {
        if self
            .thin_pool
            .set_fs_merge_scheduled(fs_uuid, new_scheduled)?
        {
            Ok(PropChangeAction::NewValue(new_scheduled))
        } else {
            Ok(PropChangeAction::Identity)
        }
    }
}

pub struct StratPoolState {
    metadata_size: Bytes,
    out_of_alloc_space: bool,
}

impl StateDiff for StratPoolState {
    type Diff = StratPoolDiff;

    fn diff(&self, other: &Self) -> Self::Diff {
        StratPoolDiff {
            metadata_size: self.metadata_size.compare(&other.metadata_size),
            out_of_alloc_space: self.out_of_alloc_space.compare(&other.out_of_alloc_space),
        }
    }

    fn unchanged(&self) -> Self::Diff {
        StratPoolDiff {
            metadata_size: Diff::Unchanged(self.metadata_size),
            out_of_alloc_space: Diff::Unchanged(self.out_of_alloc_space),
        }
    }
}

impl DumpState<'_> for StratPool {
    type State = StratPoolState;
    type DumpInput = ();

    fn cached(&self) -> Self::State {
        StratPoolState {
            metadata_size: self.metadata_size.bytes(),
            out_of_alloc_space: self.thin_pool.out_of_alloc_space(),
        }
    }

    fn dump(&mut self, _: Self::DumpInput) -> Self::State {
        self.metadata_size = self.backstore.datatier_metadata_size();
        StratPoolState {
            metadata_size: self.metadata_size.bytes(),
            out_of_alloc_space: self.thin_pool.out_of_alloc_space(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        fs::OpenOptions,
        io::{BufWriter, Read, Write},
    };

    use nix::mount::{mount, umount, MsFlags};

    use devicemapper::{Bytes, IEC, SECTOR_SIZE};

    use crate::engine::{
        strat_engine::{
            cmd::udev_settle,
            pool::AnyPool,
            tests::{crypt, loopbacked, real},
            thinpool::ThinPoolStatusDigest,
        },
        types::{EngineAction, IntegritySpec, PoolIdentifier},
        unshare_mount_namespace, Engine, StratEngine,
    };

    use super::*;

    fn invariant(pool: &StratPool, pool_name: &str) {
        check_metadata(&pool.record(&Name::new(pool_name.into()))).unwrap();
        assert!(!(pool.is_encrypted() && pool.backstore.has_cache()));
        assert!(pool
            .backstore
            .blockdevs()
            .iter()
            .all(|(_, _, bd)| bd.metadata_path().is_absolute()))
    }

    /// Test that initializing a cache causes metadata to be updated. Verify
    /// that data written before the cache was initialized can be read
    /// afterwards.
    fn test_add_cachedevs(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let devices2 = ProcessedPathInfos::try_from(paths2).unwrap();
        let (stratis_devices, unowned_devices2) = devices2.unpack();
        stratis_devices.error_on_not_empty().unwrap();

        let name = "stratis-test-pool";
        let (uuid, mut pool) = StratPool::initialize(
            name,
            unowned_devices2,
            None,
            ValidatedIntegritySpec::default(),
        )
        .unwrap();
        invariant(&pool, name);

        let metadata1 = pool.record(name);
        assert_matches!(metadata1.backstore.cache_tier, None);

        let (_, fs_uuid, _) = pool
            .create_filesystems(name, uuid, &[("stratis-filesystem", None, None)])
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
                .truncate(true)
                .write(true)
                .open(&new_file)
                .unwrap()
                .write_all(bytestring)
                .unwrap();
        }

        pool.init_cache(uuid, name, paths1, true).unwrap();
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
        pool.teardown(uuid).unwrap();
    }

    #[test]
    fn loop_test_add_cachedevs() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(3, 4, None),
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

    // Verify that it is possible to add datadevs after a cache is initialized.
    fn test_add_cachedevs_and_datadevs(paths: &[&Path]) {
        assert!(paths.len() > 2);

        let (cache_path, data_paths) = paths.split_at(1);
        let (data_path, data_paths) = data_paths.split_at(1);

        let devices = ProcessedPathInfos::try_from(data_path).unwrap();
        let (stratis_devices, unowned_devices) = devices.unpack();
        stratis_devices.error_on_not_empty().unwrap();

        let name = "stratis-test-pool";
        let (uuid, mut pool) = StratPool::initialize(
            name,
            unowned_devices,
            None,
            ValidatedIntegritySpec::default(),
        )
        .unwrap();
        invariant(&pool, name);

        pool.init_cache(uuid, name, cache_path, true).unwrap();
        invariant(&pool, name);

        pool.add_blockdevs(uuid, name, data_paths, BlockDevTier::Data)
            .unwrap();

        pool.teardown(uuid).unwrap();
    }

    #[test]
    fn loop_test_add_cachedevs_and_datadevs() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(3, 4, None),
            test_add_cachedevs_and_datadevs,
        );
    }

    #[test]
    fn real_test_add_cachedevs_and_datadevs() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(3, None, None),
            test_add_cachedevs_and_datadevs,
        );
    }

    /// Verify that adding additional blockdevs will cause a pool that is
    /// out of space to be extended.
    fn test_add_datadevs(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let (paths1, paths2) = paths.split_at(1);

        let devices1 = ProcessedPathInfos::try_from(paths1).unwrap();
        let (stratis_devices, unowned_devices1) = devices1.unpack();
        stratis_devices.error_on_not_empty().unwrap();

        let name = "stratis-test-pool";
        let (pool_uuid, mut pool) = StratPool::initialize(
            name,
            unowned_devices1,
            None,
            ValidatedIntegritySpec::default(),
        )
        .unwrap();
        invariant(&pool, name);

        let fs_name = "stratis_test_filesystem";
        let (_, fs_uuid, _) = pool
            .create_filesystems(name, pool_uuid, &[(fs_name, None, None)])
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
            while matches!(pool.thin_pool.state(), Some(ThinPoolStatusDigest::Good)) {
                f.write_all(buf).unwrap();
                amount_written += Sectors(1);
                // Run check roughly every time the buffer is cleared.
                // Running it more often is pointless as the pool is guaranteed
                // not to see any effects unless the buffer is cleared.
                if amount_written % buffer_length == Sectors(1) {
                    pool.event_on(pool_uuid, &Name::new(name.to_string()))
                        .unwrap();
                }
            }

            pool.add_blockdevs(pool_uuid, name, paths2, BlockDevTier::Data)
                .unwrap();

            let pool_diff = pool
                .event_on(pool_uuid, &Name::new(name.to_string()))
                .unwrap();

            assert!(pool_diff.thin_pool.allocated_size.is_changed());

            match pool.thin_pool.state() {
                Some(ThinPoolStatusDigest::Good) => (),
                _ => panic!("thin pool status should be back to working"),
            }
        }
        udev_settle().unwrap();
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

        let devices = ProcessedPathInfos::try_from(paths).unwrap();
        let (stratis_devices, unowned_devices) = devices.unpack();
        stratis_devices.error_on_not_empty().unwrap();

        let (uuid, mut pool) = StratPool::initialize(
            name,
            unowned_devices,
            None,
            ValidatedIntegritySpec::default(),
        )
        .unwrap();
        invariant(&pool, name);

        assert_eq!(pool.action_avail, ActionAvailability::Full);
        assert!(pool.return_rollback_failure().is_err());
        assert_eq!(pool.action_avail, ActionAvailability::NoRequests);

        pool.destroy(uuid).unwrap();
        udev_settle().unwrap();

        let name = "stratis-test-pool";

        let devices = ProcessedPathInfos::try_from(paths).unwrap();
        let (stratis_devices, unowned_devices) = devices.unpack();
        stratis_devices.error_on_not_empty().unwrap();

        let (_, mut pool) = StratPool::initialize(
            name,
            unowned_devices,
            None,
            ValidatedIntegritySpec::default(),
        )
        .unwrap();
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
    fn real_test_maintenance_mode() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(
                2,
                Some(Bytes::from(IEC::Gi * 2).sectors()),
                Some(Bytes::from(IEC::Gi * 4).sectors()),
            ),
            test_maintenance_mode,
        );
    }

    /// Test overprovisioning mode disabled and enabled assuring that the appropriate
    /// checks and behavior are in place.
    fn test_overprov(paths: &[&Path]) {
        assert!(paths.len() == 1);

        let pool_name = "pool";

        let devices = ProcessedPathInfos::try_from(paths).unwrap();
        let (stratis_devices, unowned_devices) = devices.unpack();
        stratis_devices.error_on_not_empty().unwrap();

        let (pool_uuid, mut pool) = StratPool::initialize(
            pool_name,
            unowned_devices,
            None,
            ValidatedIntegritySpec::default(),
        )
        .unwrap();

        let (_, fs_uuid, _) = pool
            .create_filesystems(
                pool_name,
                pool_uuid,
                &[(
                    "stratis_test_filesystem",
                    Some(pool.backstore.datatier_usable_size().bytes() * 2u64),
                    None,
                )],
            )
            .unwrap()
            .changed()
            .unwrap()
            .pop()
            .unwrap();
        udev_settle().unwrap();
        assert!(pool
            .set_overprov_mode(&Name::new(pool_name.to_string()), false)
            .is_err());
        pool.destroy_filesystems(pool_name, &[fs_uuid].into())
            .unwrap();

        pool.set_overprov_mode(&Name::new(pool_name.to_string()), false)
            .unwrap();
        assert!(pool
            .create_filesystems(
                pool_name,
                pool_uuid,
                &[(
                    "stratis_test_filesystem",
                    Some(pool.backstore.datatier_usable_size().bytes() * 2u64),
                    None,
                )],
            )
            .is_err());

        let mut initial_fs_size = pool.backstore.datatier_usable_size().bytes() * 2u64 / 3u64;
        initial_fs_size = initial_fs_size.sectors().bytes();
        let half_init_size = initial_fs_size / 2u64 + Bytes(1);
        let (_, fs_uuid, _) = pool
            .create_filesystems(
                pool_name,
                pool_uuid,
                &[("stratis_test_filesystem", Some(initial_fs_size), None)],
            )
            .unwrap()
            .changed()
            .unwrap()
            .pop()
            .unwrap();

        let tmp_dir = tempfile::Builder::new()
            .prefix("stratis_testing")
            .tempdir()
            .unwrap();
        let new_file = tmp_dir.path().join("stratis_test.txt");
        let sector = &[0; 512];

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
        }

        let mut f = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(new_file)
            .unwrap();
        let mut written = Sectors(0);
        while written.bytes() < half_init_size {
            f.write_all(sector).unwrap();
            written += Sectors(1);
        }
        let diffs = pool.fs_event_on(pool_uuid).unwrap();
        assert!(diffs.get(&fs_uuid).unwrap().size.is_changed());

        let (_, fs) = pool.get_filesystem(fs_uuid).unwrap();
        assert!(fs.thindev_size() < initial_fs_size.sectors() * 2u64);
    }

    #[test]
    fn loop_test_overprov() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, Some(Sectors(10 * IEC::Mi))),
            test_overprov,
        );
    }

    #[test]
    fn real_test_overprov() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, Some(Sectors(10 * IEC::Mi)), None),
            test_overprov,
        );
    }

    /// Set up for testing physical device growth.
    fn test_grow_physical_pre_grow(paths: &[&Path]) {
        let pool_name = Name::new("pool".to_string());
        let engine = StratEngine::initialize().unwrap();
        let pool_uuid =
            test_async!(engine.create_pool(&pool_name, paths, None, IntegritySpec::default()))
                .unwrap()
                .changed()
                .unwrap();
        let mut guard = test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(pool_uuid))).unwrap();
        let (_, _, pool) = guard.as_mut_tuple();

        let (_, fs_uuid, _) = pool
            .create_filesystems(
                &pool_name,
                pool_uuid,
                &[("stratis_test_filesystem", None, None)],
            )
            .unwrap()
            .changed()
            .unwrap()
            .pop()
            .unwrap();

        let tmp_dir = tempfile::Builder::new()
            .prefix("stratis_testing")
            .tempdir()
            .unwrap();
        let new_file = tmp_dir.path().join("stratis_test.txt");
        let write_block = vec![0; 512_000].into_boxed_slice();

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
        }

        let mut f = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(new_file)
            .unwrap();
        while !pool.out_of_alloc_space() {
            f.write_all(&write_block).unwrap();
            f.sync_all().unwrap();
            match pool {
                AnyPool::V1(p) => p.event_on(pool_uuid, &pool_name).unwrap(),
                AnyPool::V2(p) => p.event_on(pool_uuid, &pool_name).unwrap(),
            };
        }
    }

    /// Test that growing a physical device succeeds, the device has doubled in size,
    /// and that the pool registers new available allocation space if it is out of space
    /// at the time of device growth.
    fn test_grow_physical_post_grow(_: &[&Path]) {
        let engine = StratEngine::initialize().unwrap();

        let mut pools = test_async!(engine.pools_mut());
        assert!(pools.len() == 1);
        let (pool_name, pool_uuid, pool) = pools.iter_mut().next().unwrap();

        let (dev_uuid, size) = {
            let blockdevs = pool.blockdevs();
            let (dev_uuid, _, dev) = blockdevs.first().unwrap();
            (*dev_uuid, dev.size())
        };

        assert!(pool.out_of_alloc_space());
        let (act, pool_diff) = pool.grow_physical(pool_name, *pool_uuid, dev_uuid).unwrap();
        assert!(act.is_changed());
        let (_, dev) = pool.get_blockdev(dev_uuid).unwrap();
        assert_eq!(dev.size(), 2u64 * size);
        assert!(!pool.out_of_alloc_space());
        assert!(!pool_diff
            .unwrap()
            .pool
            .out_of_alloc_space
            .changed()
            .unwrap());
    }

    #[test]
    fn loop_test_grow_physical() {
        loopbacked::test_device_grow_with_spec(
            &loopbacked::DeviceLimits::Exactly(2, Some(Sectors(10 * IEC::Mi))),
            test_grow_physical_pre_grow,
            test_grow_physical_post_grow,
        );
    }

    /// Tests:
    /// 1. Clevis only pool creation (regression test)
    /// 2. Unbinding last token slot (should fail)
    /// 3. Binding additional clevis token
    /// 4. Binding additional keyring token
    /// 5. Keyring only pool creation
    /// 6. Unbinding last token slot (should fail)
    /// 7. Binding additional clevis token
    /// 8. Binding additional keyring token (idempotence test)
    /// 9. Pool creation with both types
    /// 10. Test that unbinding Clevis with keyring unbind and vice versa fails
    /// 11. Clevis and keyring idempotence test with specific keyslot
    /// 12. Create new Clevis binding with specific token slot
    fn clevis_test_multiple_token_slots(paths: &[&Path]) {
        fn test_multiple_token_slots_with_key(paths: &[&Path], key_desc: &KeyDescription) {
            unshare_mount_namespace().unwrap();

            let engine = StratEngine::initialize().unwrap();

            let first = paths[0];
            let pool_uuid_clevis_only = test_async!(engine.create_pool(
                "clevis_only",
                &[first],
                InputEncryptionInfo::new(
                    vec![],
                    vec![(Some(0), (
                        "tang".to_string(),
                        json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
                    ))],
                ).unwrap().as_ref(),
                IntegritySpec::default(),
            )).unwrap().changed().unwrap();

            {
                let mut handle_clevis_only =
                    test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(pool_uuid_clevis_only)))
                        .unwrap();

                assert!(handle_clevis_only.unbind_clevis(Some(0)).is_err());
                handle_clevis_only
                    .bind_clevis(
                        OptionalTokenSlotInput::None,
                        "tang",
                        &json!({
                            "url": env::var("TANG_URL").expect("TANG_URL env var required"),
                            "stratis:tang:trust_url": true,
                        }),
                    )
                    .unwrap();
                handle_clevis_only
                    .bind_keyring(OptionalTokenSlotInput::None, key_desc)
                    .unwrap();
            }

            test_async!(engine.destroy_pool(pool_uuid_clevis_only)).unwrap();

            let second = paths[1];
            let pool_uuid_keyring_only = test_async!(engine.create_pool(
                "keyring_only",
                &[second],
                InputEncryptionInfo::new(vec![(Some(0), key_desc.to_owned())], vec![])
                    .unwrap()
                    .as_ref(),
                IntegritySpec::default(),
            ))
            .unwrap()
            .changed()
            .unwrap();

            {
                let mut handle_keyring_only =
                    test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(pool_uuid_keyring_only)))
                        .unwrap();

                assert!(handle_keyring_only.unbind_keyring(Some(0)).is_err());
                handle_keyring_only
                    .bind_clevis(
                        OptionalTokenSlotInput::None,
                        "tang",
                        &json!({
                            "url": env::var("TANG_URL").expect("TANG_URL env var required"),
                            "stratis:tang:trust_url": true,
                        }),
                    )
                    .unwrap();
                matches!(
                    handle_keyring_only.bind_keyring(OptionalTokenSlotInput::None, key_desc),
                    Ok(CreateAction::Identity)
                );
            }

            test_async!(engine.destroy_pool(pool_uuid_keyring_only)).unwrap();

            let third = paths[2];
            let pool_uuid_both = test_async!(engine.create_pool(
                "both",
                &[third],
                InputEncryptionInfo::new(
                    vec![(Some(0), key_desc.to_owned())],
                    vec![(Some(1), (
                        "tang".to_string(),
                        json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
                    ))],
                )
                .unwrap()
                .as_ref(),
                IntegritySpec::default(),
            ))
            .unwrap()
            .changed()
            .unwrap();

            {
                let mut handle_both =
                    test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(pool_uuid_both))).unwrap();

                assert!(handle_both.unbind_keyring(Some(1)).is_err());
                assert!(handle_both.unbind_clevis(Some(0)).is_err());
                matches!(
                    handle_both.bind_clevis(
                        OptionalTokenSlotInput::Some(1),
                        "tang",
                        &json!({
                            "url": env::var("TANG_URL").expect("TANG_URL env var required"),
                            "stratis:tang:trust_url": true,
                        }),
                    ),
                    Ok(CreateAction::Identity)
                );
                matches!(
                    handle_both.bind_keyring(OptionalTokenSlotInput::Some(0), key_desc),
                    Ok(CreateAction::Identity)
                );
                matches!(
                    handle_both.bind_keyring(OptionalTokenSlotInput::Some(2), key_desc,),
                    Ok(CreateAction::Created(_))
                );
            }

            test_async!(engine.destroy_pool(pool_uuid_both)).unwrap();
        }

        crypt::insert_and_cleanup_key(paths, test_multiple_token_slots_with_key);
    }

    #[test]
    fn clevis_loop_test_multiple_token_slots() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(3, None),
            clevis_test_multiple_token_slots,
        );
    }

    #[test]
    fn clevis_real_test_multiple_token_slots() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(3, None, None),
            clevis_test_multiple_token_slots,
        );
    }
}

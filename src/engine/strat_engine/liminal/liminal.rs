// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Management of devices which are known to stratisd but not in a pool.

use std::{
    collections::{HashMap, HashSet},
    os::fd::RawFd,
    path::PathBuf,
};

use chrono::{DateTime, Utc};
use either::Either;
use libcryptsetup_rs::SafeMemHandle;
use serde_json::{Map, Value};

use devicemapper::Sectors;

use crate::{
    engine::{
        engine::{DumpState, Pool, StateDiff, MAX_STRATIS_PASS_SIZE},
        shared::read_key_shared,
        strat_engine::{
            backstore::{blockdev::InternalBlockDev, find_stratis_devs_by_uuid},
            crypt::handle::v1::CryptHandle,
            dm::{
                has_leftover_devices, has_leftover_devices_legacy, stop_partially_constructed_pool,
                stop_partially_constructed_pool_legacy,
            },
            liminal::{
                device_info::{stratis_infos_ref, DeviceSet, LInfo, LLuksInfo, LStratisInfo},
                identify::{
                    bda_wrapper, identify_block_device, DeviceInfo, LuksInfo, StratisDevInfo,
                    StratisInfo,
                },
                setup::{get_blockdevs, get_blockdevs_legacy, get_metadata},
            },
            metadata::StratisIdentifiers,
            pool::{v1, v2, AnyPool},
            serde_structs::{PoolFeatures, PoolSave},
        },
        structures::Table,
        types::{
            DevUuid, LockedPoolsInfo, MaybeInconsistent, Name, PoolEncryptionInfo, PoolIdentifier,
            PoolUuid, SizedKeyMemory, StoppedPoolsInfo, StratBlockDevDiff, StratSigblockVersion,
            TokenUnlockMethod, UdevEngineEvent, UuidOrConflict,
        },
        BlockDevTier,
    },
    stratis::{StratisError, StratisResult},
};

type MetadataResult = StratisResult<(
    DateTime<Utc>,
    PoolSave,
    Option<Vec<Either<PathBuf, PathBuf>>>,
)>;

/// Devices which stratisd has discovered but which have not been assembled
/// into pools.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct LiminalDevices {
    /// Lookup data structure for pool and device UUIDs corresponding with
    /// a path where the superblock was either removed or the device was removed.
    uuid_lookup: HashMap<PathBuf, (PoolUuid, DevUuid)>,
    /// Devices that have not yet been set up or have been stopped.
    stopped_pools: HashMap<PoolUuid, DeviceSet>,
    /// Devices that have been left in a partially constructed state either during start
    /// or stop.
    partially_constructed_pools: HashMap<PoolUuid, DeviceSet>,
    /// Lookup data structure for name to UUID mapping for starting pools by name.
    name_to_uuid: HashMap<Name, UuidOrConflict>,
}

impl LiminalDevices {
    #[allow(dead_code)]
    fn invariant(&self) {
        assert!(
            self.stopped_pools
                .keys()
                .cloned()
                .collect::<HashSet<_>>()
                .difference(
                    &self
                        .uuid_lookup
                        .iter()
                        .map(|(_, (u, _))| *u)
                        .collect::<HashSet<_>>()
                )
                .count()
                == 0
        );
    }

    // Unlock the liminal encrypted devices that correspond to the given pool UUID.
    fn unlock_pool(
        &mut self,
        pools: &Table<PoolUuid, AnyPool>,
        pool_uuid: PoolUuid,
        token_slot: TokenUnlockMethod,
        passphrase: Option<&SizedKeyMemory>,
    ) -> StratisResult<Vec<DevUuid>> {
        fn handle_luks(
            luks_info: &LLuksInfo,
            token_slot: TokenUnlockMethod,
            passphrase: Option<&SizedKeyMemory>,
        ) -> StratisResult<()> {
            if CryptHandle::setup(&luks_info.dev_info.devnode, token_slot, passphrase)?.is_some() {
                Ok(())
            } else {
                Err(StratisError::Msg(format!(
                    "Block device {} does not appear to be formatted with
                        the proper Stratis LUKS2 metadata.",
                    luks_info.dev_info.devnode.display(),
                )))
            }
        }

        let unlocked = match self
            .stopped_pools
            .get(&pool_uuid)
            .or_else(|| self.partially_constructed_pools.get(&pool_uuid))
        {
            Some(map) => {
                let encryption_info = map.encryption_info();
                if let Ok(None) = encryption_info {
                    return Err(StratisError::Msg(
                        format!(
                            "Attempted to unlock set of devices belonging to an unencrypted pool with UUID {pool_uuid}"
                        ),
                    ));
                } else if let Err(e) = encryption_info {
                    return Err(StratisError::Chained(
                        format!(
                            "Error in the encryption information for pool with UUID {pool_uuid}"
                        ),
                        Box::new(e),
                    ));
                }

                let mut unlocked = Vec::new();
                for (dev_uuid, info) in map.iter() {
                    match info {
                        LInfo::Stratis(_) => (),
                        LInfo::Luks(ref luks_info) => {
                            match handle_luks(luks_info, token_slot, passphrase) {
                                Ok(()) => unlocked.push(*dev_uuid),
                                Err(e) => return Err(e),
                            }
                        }
                    }
                }
                unlocked
            }
            None => match pools.get_by_uuid(pool_uuid) {
                Some((_, pool)) => {
                    if pool.is_encrypted() {
                        vec![]
                    } else {
                        return Err(StratisError::Msg(format!(
                            "Pool with UUID {pool_uuid} is not encrypted and cannot be unlocked."
                        )));
                    }
                }
                None => {
                    return Err(StratisError::Msg(format!(
                        "No devices with UUID {pool_uuid} have been registered with stratisd."
                    )))
                }
            },
        };

        Ok(unlocked)
    }

    /// Start a pool, create the devicemapper devices, and return the fully constructed
    /// legacy pool.
    ///
    /// Precondition: Pool was determined to be in stopped or partially constructed pools.
    pub fn start_pool_legacy(
        &mut self,
        pools: &Table<PoolUuid, AnyPool>,
        pool_uuid: PoolUuid,
        token_slot: TokenUnlockMethod,
        passphrase_fd: Option<RawFd>,
        remove_cache: bool,
    ) -> StratisResult<(Name, PoolUuid, AnyPool, Vec<DevUuid>)> {
        fn start_pool_failure(
            pools: &Table<PoolUuid, AnyPool>,
            pool_uuid: PoolUuid,
            luks_info: StratisResult<Option<PoolEncryptionInfo>>,
            infos: &HashMap<DevUuid, Box<LStratisInfo>>,
            meta_res: MetadataResult,
        ) -> StratisResult<(Name, AnyPool)> {
            let (timestamp, metadata, paths_to_wipe) = meta_res?;

            setup_pool_legacy(
                pools,
                pool_uuid,
                luks_info,
                infos,
                timestamp,
                metadata,
                paths_to_wipe,
            )
        }

        let pool = self
            .stopped_pools
            .get(&pool_uuid)
            .or_else(|| self.partially_constructed_pools.get(&pool_uuid))
            .expect("Checked in caller");

        // Here we take a reference to entries in stopped pools because the call to unlock_pool
        // below requires the pool being unlocked to still have its entry in stopped_pools.
        // Removing it here would cause an error.
        let encryption_info = pool.encryption_info();
        let unlocked_devices = match (encryption_info, token_slot, passphrase_fd) {
            (Ok(None), TokenUnlockMethod::None, None) => Vec::new(),
            (Ok(None), _, _) => {
                return Err(StratisError::Msg(format!(
                    "Pool with UUID {pool_uuid} is not encrypted but an unlock method or passphrase was provided"
                )));
            }
            (Ok(Some(_)), TokenUnlockMethod::None, _) => {
                return Err(StratisError::Msg(format!(
                    "Pool with UUID {pool_uuid} is encrypted but no unlock method was provided"
                )));
            }
            (Ok(Some(_)), method, passphrase_fd) => {
                let passphrase = if let Some(fd) = passphrase_fd {
                    let mut memory = SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE)?;
                    let len = read_key_shared(fd, memory.as_mut())?;
                    Some(SizedKeyMemory::new(memory, len))
                } else {
                    None
                };

                self.unlock_pool(pools, pool_uuid, method, passphrase.as_ref())?
            }
            (Err(e), _, _) => return Err(e),
        };

        let uuids = unlocked_devices.into_iter().collect::<Vec<_>>();

        let mut stopped_pool = self
            .stopped_pools
            .remove(&pool_uuid)
            .or_else(|| self.partially_constructed_pools.remove(&pool_uuid))
            .expect("Checked above");
        match find_stratis_devs_by_uuid(pool_uuid, &uuids) {
            Ok(infos) => infos.into_iter().for_each(|(dev_uuid, (path, devno))| {
                if let Ok(Ok(Some(bda))) = bda_wrapper(&path) {
                    self.uuid_lookup
                        .insert(path.to_path_buf(), (pool_uuid, dev_uuid));

                    stopped_pool.process_info_add(DeviceInfo::Stratis(StratisInfo {
                        dev_info: StratisDevInfo {
                            device_number: devno,
                            devnode: path.to_path_buf(),
                        },
                        bda,
                    }));
                } else {
                    warn!(
                        "Failed to read BDA of device with pool UUID {pool_uuid}, dev UUID, {dev_uuid}; ignoring"
                    );
                }
            }),
            Err(e) => {
                warn!("Failed to scan for newly unlocked Stratis devices: {e}");
                return Err(e);
            }
        }

        assert!(pools.get_by_uuid(pool_uuid).is_none());
        assert!(!self.stopped_pools.contains_key(&pool_uuid));

        let luks_info = stopped_pool.encryption_info();
        let mut infos = match stopped_pool.into_opened_set() {
            Either::Left(i) => i,
            Either::Right(ds) => {
                let err = StratisError::Msg(format!(
                    "Some of the devices in pool with UUID {pool_uuid} are unopened"
                ));
                info!("Attempt to set up pool failed, but it may be possible to set up the pool later, if the situation changes: {err}");
                self.handle_stopped_pool(pool_uuid, ds);
                return Err(err);
            }
        };

        let res = load_stratis_metadata(pool_uuid, &mut infos, remove_cache);

        match start_pool_failure(pools, pool_uuid, luks_info, &infos, res) {
            Ok((name, pool)) => {
                self.uuid_lookup = self
                    .uuid_lookup
                    .drain()
                    .filter(|(_, (p, _))| *p != pool_uuid)
                    .collect();
                self.name_to_uuid = self
                    .name_to_uuid
                    .drain()
                    .filter_map(|(n, mut maybe_conflict)| {
                        if maybe_conflict.remove(&pool_uuid) {
                            None
                        } else {
                            Some((n, maybe_conflict))
                        }
                    })
                    .collect();
                info!("Pool with name \"{name}\" and UUID \"{pool_uuid}\" set up");
                Ok((name, pool_uuid, pool, uuids))
            }
            Err(err) => {
                info!("Attempt to set up pool failed, but it may be possible to set up the pool later, if the situation changes: {err}");
                self.handle_stopped_pool(
                    pool_uuid,
                    infos
                        .into_iter()
                        .map(|(uuid, info)| (uuid, LInfo::Stratis(info)))
                        .collect::<DeviceSet>(),
                );
                Err(err)
            }
        }
    }

    /// Start a pool, create the devicemapper devices, and return the fully constructed
    /// metadata V2 pool.
    ///
    /// Precondition: Pool was determined to be in stopped or partially constructed pools.
    pub fn start_pool_new(
        &mut self,
        pools: &Table<PoolUuid, AnyPool>,
        pool_uuid: PoolUuid,
        token_slot: TokenUnlockMethod,
        passphrase_fd: Option<RawFd>,
        remove_cache: bool,
    ) -> StratisResult<(Name, PoolUuid, AnyPool, Vec<DevUuid>)> {
        fn start_pool_failure(
            pools: &Table<PoolUuid, AnyPool>,
            pool_uuid: PoolUuid,
            infos: &HashMap<DevUuid, Box<LStratisInfo>>,
            meta_res: MetadataResult,
            token_slot: TokenUnlockMethod,
            passphrase_fd: Option<RawFd>,
        ) -> StratisResult<(Name, AnyPool)> {
            let (timestamp, metadata, paths_to_wipe) = meta_res?;
            let processed_paths_to_wipe = paths_to_wipe.map(|v| {
                v.into_iter()
                    .map(|e| {
                        e.left()
                            .expect("Must be an unencrypted path in V2 of the metadata")
                    })
                    .collect::<Vec<_>>()
            });

            let passphrase = match (
                metadata.features.contains(&PoolFeatures::Encryption),
                token_slot,
                passphrase_fd,
            ) {
                (_, _, None) => None,
                (false, _, _) => {
                    return Err(StratisError::Msg(format!(
                        "Pool with UUID {pool_uuid} is not encrypted but an unlock method or passphrase was provided"
                    )));
                }
                (true, TokenUnlockMethod::None, _) => return Err(StratisError::Msg(
                    "Metadata reported that encryption enabled but no unlock method was provided"
                        .to_string(),
                )),
                (true, _, Some(fd)) => {
                    let mut memory = match SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE) {
                        Ok(m) => m,
                        Err(e) => return Err(StratisError::from(e)),
                    };
                    let len = read_key_shared(fd, memory.as_mut())?;
                    Some(SizedKeyMemory::new(memory, len))
                }
            };

            setup_pool(
                pools,
                pool_uuid,
                infos,
                timestamp,
                metadata,
                processed_paths_to_wipe,
                token_slot,
                passphrase,
            )
        }

        let stopped_pool = self
            .stopped_pools
            .remove(&pool_uuid)
            .or_else(|| self.partially_constructed_pools.remove(&pool_uuid))
            .expect("Checked above");

        assert!(pools.get_by_uuid(pool_uuid).is_none());
        assert!(!self.stopped_pools.contains_key(&pool_uuid));

        let mut infos = stopped_pool
            .into_opened_set()
            .expect_left("Cannot fail in V2 of metadata");

        let res = load_stratis_metadata(pool_uuid, &mut infos, remove_cache);

        match start_pool_failure(pools, pool_uuid, &infos, res, token_slot, passphrase_fd) {
            Ok((name, pool)) => {
                self.uuid_lookup = self
                    .uuid_lookup
                    .drain()
                    .filter(|(_, (p, _))| *p != pool_uuid)
                    .collect();
                self.name_to_uuid = self
                    .name_to_uuid
                    .drain()
                    .filter_map(|(n, mut maybe_conflict)| {
                        if maybe_conflict.remove(&pool_uuid) {
                            None
                        } else {
                            Some((n, maybe_conflict))
                        }
                    })
                    .collect();
                info!("Pool with name \"{name}\" and UUID \"{pool_uuid}\" set up");
                Ok((name, pool_uuid, pool, Vec::new()))
            }
            Err(err) => {
                info!("Attempt to set up pool failed, but it may be possible to set up the pool later, if the situation changes: {err}");
                self.handle_stopped_pool(
                    pool_uuid,
                    infos
                        .into_iter()
                        .map(|(uuid, info)| (uuid, LInfo::Stratis(info)))
                        .collect::<DeviceSet>(),
                );
                Err(err)
            }
        }
    }

    /// Start a pool, create the devicemapper devices, and return the fully constructed
    /// pool.
    pub fn start_pool(
        &mut self,
        pools: &Table<PoolUuid, AnyPool>,
        id: PoolIdentifier<PoolUuid>,
        token_slot: TokenUnlockMethod,
        passphrase_fd: Option<RawFd>,
        remove_cache: bool,
    ) -> StratisResult<(Name, PoolUuid, AnyPool, Vec<DevUuid>)> {
        let pool_uuid = match id {
            PoolIdentifier::Uuid(u) => u,
            PoolIdentifier::Name(ref n) => self
                .name_to_uuid
                .get(n)
                .ok_or_else(|| StratisError::Msg(format!("Could not find a pool with name {n}")))
                .and_then(|uc| uc.to_result())?,
        };
        let pool = self
            .stopped_pools
            .get(&pool_uuid)
            .or_else(|| self.partially_constructed_pools.get(&pool_uuid))
            .ok_or_else(|| {
                StratisError::Msg(format!(
                    "Requested pool with UUID {pool_uuid} was not found in stopped or partially constructed pools"
                ))
            })?;
        let metadata_version = pool.metadata_version()?;

        match metadata_version {
            StratSigblockVersion::V1 => {
                self.start_pool_legacy(pools, pool_uuid, token_slot, passphrase_fd, remove_cache)
            }
            StratSigblockVersion::V2 => {
                self.start_pool_new(pools, pool_uuid, token_slot, passphrase_fd, remove_cache)
            }
        }
    }

    /// Stop a pool, tear down the devicemapper devices, and store the pool information
    /// in an internal data structure for later starting.
    /// Returns true if the pool was torn down entirely, false if the pool is
    /// partially up. Returns an error if the pool has some untorndown
    /// filesystems, as in that case the pool needs to be administered.
    pub fn stop_pool(
        &mut self,
        pools: &mut Table<PoolUuid, AnyPool>,
        pool_name: Name,
        pool_uuid: PoolUuid,
        mut pool: AnyPool,
    ) -> StratisResult<bool> {
        let res = match pool {
            AnyPool::V1(ref mut p) => p.stop(&pool_name, pool_uuid),
            AnyPool::V2(ref mut p) => p.stop(&pool_name, pool_uuid),
        };
        let (devices, err) = match res {
            Ok(devs) => (devs, None),
            Err((e, true)) => {
                pools.insert(pool_name, pool_uuid, pool);
                return Err(e);
            }
            Err((e, false)) => {
                warn!("Failed to stop pool; placing in partially constructed pools");
                (
                    match pool {
                        AnyPool::V1(ref mut p) => DeviceSet::from(
                            p.blockdevs()
                                .into_iter()
                                .map(|(_, _, bd)| bd)
                                .collect::<Vec<_>>(),
                        ),
                        AnyPool::V2(ref mut p) => DeviceSet::from(
                            p.blockdevs()
                                .into_iter()
                                .map(|(_, _, bd)| bd)
                                .collect::<Vec<_>>(),
                        ),
                    },
                    Some(e),
                )
            }
        };
        for (_, device) in devices.iter() {
            match device {
                LInfo::Luks(l) => {
                    self.uuid_lookup.insert(
                        l.dev_info.devnode.clone(),
                        (l.identifiers.pool_uuid, l.identifiers.device_uuid),
                    );
                }
                LInfo::Stratis(s) => {
                    self.uuid_lookup.insert(
                        s.dev_info.devnode.clone(),
                        (
                            s.bda.identifiers().pool_uuid,
                            s.bda.identifiers().device_uuid,
                        ),
                    );
                }
            }
        }
        if err.is_some() {
            self.partially_constructed_pools.insert(pool_uuid, devices);
        } else {
            self.stopped_pools.insert(pool_uuid, devices);
        }
        if let Some(maybe_conflict) = self.name_to_uuid.get_mut(&pool_name) {
            maybe_conflict.add(pool_uuid);
            if let UuidOrConflict::Conflict(set) = maybe_conflict {
                warn!("Found conflicting names for stopped pools; UUID will be required to start pools with UUIDs {}", set.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(", "));
            }
        } else {
            self.name_to_uuid
                .insert(pool_name.clone(), UuidOrConflict::Uuid(pool_uuid));
        }
        Ok(err.is_none())
    }

    /// Tear down a partially constructed pool.
    pub fn stop_partially_constructed_pool(&mut self, pool_uuid: PoolUuid) -> StratisResult<()> {
        if let Some(device_set) = self.partially_constructed_pools.remove(&pool_uuid) {
            let metadata_version = device_set.metadata_version()?;
            match metadata_version {
                StratSigblockVersion::V1 => {
                    match stop_partially_constructed_pool_legacy(
                        pool_uuid,
                        &device_set
                            .iter()
                            .map(|(dev_uuid, _)| *dev_uuid)
                            .collect::<Vec<_>>(),
                    ) {
                        Ok(_) => {
                            self.stopped_pools.insert(pool_uuid, device_set);
                            Ok(())
                        }
                        Err(e) => {
                            warn!("Failed to stop partially constructed pool: {e}");
                            self.partially_constructed_pools
                                .insert(pool_uuid, device_set);
                            Err(e)
                        }
                    }
                }
                StratSigblockVersion::V2 => match stop_partially_constructed_pool(pool_uuid) {
                    Ok(_) => {
                        self.stopped_pools.insert(pool_uuid, device_set);
                        Ok(())
                    }
                    Err(e) => {
                        warn!("Failed to stop partially constructed pool: {e}");
                        self.partially_constructed_pools
                            .insert(pool_uuid, device_set);
                        Err(e)
                    }
                },
            }
        } else {
            Ok(())
        }
    }

    /// Get a mapping of pool UUIDs from all of the LUKS2 devices that are currently
    /// locked to their encryption info in the set of pools that are not yet set up.
    pub fn locked_pools(&self) -> LockedPoolsInfo {
        LockedPoolsInfo {
            name_to_uuid: self
                .name_to_uuid
                .iter()
                .filter_map(|(name, maybe_conflict)| {
                    maybe_conflict
                        .to_result()
                        .ok()
                        .map(|uuid| (name.clone(), uuid))
                })
                .collect::<HashMap<_, _>>(),
            uuid_to_name: self
                .name_to_uuid
                .iter()
                .filter_map(|(name, maybe_conflict)| {
                    maybe_conflict
                        .to_result()
                        .ok()
                        .map(|uuid| (uuid, name.clone()))
                })
                .collect::<HashMap<_, _>>(),
            locked: self
                .stopped_pools
                .iter()
                .filter_map(|(pool_uuid, map)| {
                    map.locked_pool_info().map(|info| (*pool_uuid, info))
                })
                .collect(),
        }
    }

    /// Get a mapping of pool UUIDs to device sets for all stopped pools.
    pub fn stopped_pools(&self) -> StoppedPoolsInfo {
        StoppedPoolsInfo {
            name_to_uuid: self
                .name_to_uuid
                .iter()
                .filter_map(|(name, maybe_conflict)| {
                    maybe_conflict
                        .to_result()
                        .ok()
                        .map(|uuid| (name.clone(), uuid))
                })
                .collect::<HashMap<_, _>>(),
            uuid_to_name: self
                .name_to_uuid
                .iter()
                .filter_map(|(name, maybe_conflict)| {
                    maybe_conflict
                        .to_result()
                        .ok()
                        .map(|uuid| (uuid, name.clone()))
                })
                .collect::<HashMap<_, _>>(),
            stopped: self
                .stopped_pools
                .iter()
                .filter_map(|(pool_uuid, map)| {
                    map.stopped_pool_info().map(|info| (*pool_uuid, info))
                })
                .collect(),
            partially_constructed: self
                .partially_constructed_pools
                .iter()
                .filter_map(|(pool_uuid, map)| {
                    map.stopped_pool_info().map(|info| (*pool_uuid, info))
                })
                .collect(),
        }
    }

    /// Calculate whether block device size has changed.
    fn handle_size_change<'a, B>(
        tier: BlockDevTier,
        dev_uuid: DevUuid,
        dev: &mut B,
    ) -> Option<(DevUuid, <<B as DumpState<'a>>::State as StateDiff>::Diff)>
    where
        B: DumpState<'a, DumpInput = Sectors> + InternalBlockDev,
    {
        if tier == BlockDevTier::Data {
            let orig = dev.cached();
            match dev.calc_new_size() {
                Ok(Some(s)) => Some((dev_uuid, orig.diff(&dev.dump(s)))),
                Err(e) => {
                    warn!(
                        "Failed to determine device size for {}: {}",
                        dev.physical_path().display(),
                        e
                    );
                    None
                }
                _ => None,
            }
        } else {
            None
        }
    }

    /// Take maps of pool UUIDs to sets of devices and return a list of
    /// information about created pools.
    ///
    /// Precondition: No pools have yet been set up, i.e., it is unnecessary
    /// to check for membership in any of the existing categories of device
    /// sets.
    pub fn setup_pools(
        &mut self,
        all_devices: (
            HashMap<PoolUuid, Vec<LuksInfo>>,
            HashMap<PoolUuid, Vec<StratisInfo>>,
        ),
    ) -> Vec<(Name, PoolUuid, AnyPool)> {
        let table = Table::default();
        let (mut luks_devices, mut stratis_devices) = all_devices;

        let pool_uuids: HashSet<PoolUuid> = luks_devices
            .keys()
            .cloned()
            .collect::<HashSet<PoolUuid>>()
            .union(&stratis_devices.keys().cloned().collect())
            .cloned()
            .collect();

        pool_uuids
            .iter()
            .filter_map(|pool_uuid| {
                let luks_infos = luks_devices.remove(pool_uuid);
                let stratis_infos = stratis_devices.remove(pool_uuid);
                let infos: Vec<DeviceInfo> = stratis_infos
                    .unwrap_or_default()
                    .drain(..)
                    .map(DeviceInfo::Stratis)
                    .chain(
                        luks_infos
                            .unwrap_or_default()
                            .drain(..)
                            .map(DeviceInfo::Luks),
                    )
                    .collect();

                let mut info_map = DeviceSet::new();
                for info in infos {
                    match &info {
                        DeviceInfo::Luks(l) => {
                            self.uuid_lookup.insert(
                                l.dev_info.devnode.clone(),
                                (l.identifiers.pool_uuid, l.identifiers.device_uuid),
                            );
                        }
                        DeviceInfo::Stratis(s) => {
                            self.uuid_lookup.insert(
                                s.dev_info.devnode.clone(),
                                (s.bda.identifiers().pool_uuid, s.bda.identifiers().device_uuid),
                            );
                        }
                    }

                    info_map.process_info_add(info);
                }

                match info_map.pool_level_metadata_info() {
                    Ok((MaybeInconsistent::No(Some(name)), _)) => {
                        if let Some(maybe_conflict) = self.name_to_uuid.get_mut(&name) {
                            maybe_conflict.add(*pool_uuid);
                            if let UuidOrConflict::Conflict(set) = maybe_conflict {
                                warn!("Found conflicting names for stopped pools; UUID will be required to start pools with UUIDs {}", set.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(", "));
                            }
                        } else {
                            self.name_to_uuid
                                .insert(name, UuidOrConflict::Uuid(*pool_uuid));
                        }
                    },
                    Err(e) => {
                        info!("Error while attempting to determine pool name for pool with UUID {pool_uuid}: {e}; this may resolve when more devices appear and are processed");
                    }
                    _ => (),
                }

                self.try_setup_started_pool(&table, *pool_uuid, info_map)
                    .map(|(pool_name, mut pool)| {
                        match pool {
                            AnyPool::V1(ref mut p) => {
                                match p.blockdevs_mut() {
                                    Ok(blockdevs) => {
                                        for (dev_uuid, tier, blockdev) in blockdevs {
                                            if let Some(size) =
                                                Self::handle_size_change(tier, dev_uuid, blockdev)
                                                    .and_then(|(_, d)| d.size.changed())
                                                    .and_then(|c| c)
                                            {
                                                blockdev.set_new_size(size);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Failed to check size of block devices in newly set up pool: {e}");
                                    }
                                }
                                (pool_name, *pool_uuid, pool)
                            },
                            AnyPool::V2(ref mut p) => {
                                match p.blockdevs_mut() {
                                    Ok(blockdevs) => {
                                        for (dev_uuid, tier, blockdev) in blockdevs {
                                            if let Some(size) =
                                                Self::handle_size_change(tier, dev_uuid, blockdev)
                                                    .and_then(|(_, d)| d.size.changed())
                                                    .and_then(|c| c)
                                            {
                                                blockdev.set_new_size(size);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Failed to check size of block devices in newly set up pool: {e}");
                                    }
                                }
                                (pool_name, *pool_uuid, pool)
                            },
                        }
                    })
            })
            .collect::<Vec<(Name, PoolUuid, AnyPool)>>()
    }

    /// Variation on try_setup_pool that returns None if the pool is marked
    /// as stopped in its metadata.
    ///
    /// Precondition: pools.get_by_uuid(pool_uuid).is_none() &&
    ///               self.stopped_pools.get(pool_uuid).is_none()
    fn try_setup_started_pool(
        &mut self,
        pools: &Table<PoolUuid, AnyPool>,
        pool_uuid: PoolUuid,
        device_set: DeviceSet,
    ) -> Option<(Name, AnyPool)> {
        fn try_setup_started_pool_failure(
            pools: &Table<PoolUuid, AnyPool>,
            pool_uuid: PoolUuid,
            luks_info: StratisResult<Option<PoolEncryptionInfo>>,
            infos: &HashMap<DevUuid, Box<LStratisInfo>>,
            metadata_version: StratisResult<StratSigblockVersion>,
            meta_res: StratisResult<(DateTime<Utc>, PoolSave)>,
        ) -> StratisResult<Option<(Name, AnyPool)>> {
            let metadata_version = metadata_version?;
            let (timestamp, metadata) = meta_res?;
            if let Some(true) | None = metadata.started {
                match metadata_version {
                    StratSigblockVersion::V1 => setup_pool_legacy(
                        pools, pool_uuid, luks_info, infos, timestamp, metadata, None,
                    )
                    .map(Some),
                    StratSigblockVersion::V2 => setup_pool(
                        pools,
                        pool_uuid,
                        infos,
                        timestamp,
                        metadata,
                        None,
                        TokenUnlockMethod::None,
                        None,
                    )
                    .map(Some),
                }
            } else {
                Ok(None)
            }
        }

        assert!(pools.get_by_uuid(pool_uuid).is_none());
        assert!(!self.stopped_pools.contains_key(&pool_uuid));

        let metadata_version = device_set.metadata_version();
        let luks_info = device_set.encryption_info();
        let mut infos = match device_set.into_opened_set() {
            Either::Left(i) => i,
            Either::Right(ds) => {
                let err = StratisError::Msg(format!(
                    "Some of the devices in pool with UUID {pool_uuid} are unopened"
                ));
                info!("Attempt to set up pool failed, but it may be possible to set up the pool later, if the situation changes: {err}");
                self.handle_stopped_pool(pool_uuid, ds);
                return None;
            }
        };

        let res = load_stratis_metadata(pool_uuid, &mut infos, false).map(
            |(timestamp, metadata, paths)| {
                assert!(paths.is_none());
                (timestamp, metadata)
            },
        );
        match try_setup_started_pool_failure(
            pools,
            pool_uuid,
            luks_info,
            &infos,
            metadata_version,
            res,
        ) {
            Ok(Some((name, pool))) => {
                self.uuid_lookup = self
                    .uuid_lookup
                    .drain()
                    .filter(|(_, (p, _))| *p != pool_uuid)
                    .collect();
                self.name_to_uuid = self
                    .name_to_uuid
                    .drain()
                    .filter_map(|(n, mut maybe_conflict)| {
                        if maybe_conflict.remove(&pool_uuid) {
                            None
                        } else {
                            Some((n, maybe_conflict))
                        }
                    })
                    .collect();
                info!("Pool with name \"{name}\" and UUID \"{pool_uuid}\" set up");
                Some((name, pool))
            }
            Ok(None) => {
                self.handle_stopped_pool(
                    pool_uuid,
                    infos
                        .into_iter()
                        .map(|(uuid, info)| (uuid, LInfo::Stratis(info)))
                        .collect::<DeviceSet>(),
                );
                None
            }
            Err(err) => {
                info!("Attempt to set up pool failed, but it may be possible to set up the pool later, if the situation changes: {err}");
                self.handle_stopped_pool(
                    pool_uuid,
                    infos
                        .into_iter()
                        .map(|(uuid, info)| (uuid, LInfo::Stratis(info)))
                        .collect::<DeviceSet>(),
                );
                None
            }
        }
    }

    /// On udev events, stratisd checks whether block devices have changed in size.
    /// This allows us to update the user with the new size if it has changed.
    /// This method runs a check on device size on each udev change or add event
    /// to determine whether the size has indeed changed so we can update it in
    /// our internal data structures.
    pub fn block_evaluate_size(
        pools: &mut Table<PoolUuid, AnyPool>,
        event: &UdevEngineEvent,
    ) -> StratisResult<Option<(DevUuid, StratBlockDevDiff)>> {
        let mut ret = None;

        let event_type = event.event_type();
        let device_path = match event.device().devnode() {
            Some(d) => d,
            None => return Ok(None),
        };
        let device_info = match event_type {
            libudev::EventType::Add | libudev::EventType::Change => {
                if device_path.exists() {
                    identify_block_device(event)
                } else {
                    None
                }
            }
            _ => None,
        };

        if event_type == libudev::EventType::Add || event_type == libudev::EventType::Change {
            if let Some(di) = device_info {
                let pool_uuid = di.stratis_identifiers().pool_uuid;
                let dev_uuid = di.stratis_identifiers().device_uuid;
                if let Some((_, pool)) = pools.get_mut_by_uuid(pool_uuid) {
                    match pool {
                        AnyPool::V1(p) => {
                            if let Some((tier, dev)) = p.get_mut_strat_blockdev(dev_uuid)? {
                                ret = Self::handle_size_change(tier, dev_uuid, dev);
                            }
                        }
                        AnyPool::V2(p) => {
                            if let Some((tier, dev)) = p.get_mut_strat_blockdev(dev_uuid)? {
                                ret = Self::handle_size_change(tier, dev_uuid, dev);
                            }
                        }
                    }
                }
            }
        }

        Ok(ret)
    }

    /// Given some information gathered about a single Stratis device, determine
    /// whether or not a pool can be constructed, and if it can, construct the
    /// pool and return the newly constructed pool. If the device appears to
    /// belong to a pool that has already been set up assume that no further
    /// processing is required and return None. If there is an error
    /// constructing the pool, retain the set of devices.
    pub fn block_evaluate(
        &mut self,
        pools: &Table<PoolUuid, AnyPool>,
        event: &UdevEngineEvent,
    ) -> Option<(Name, PoolUuid, AnyPool)> {
        let event_type = event.event_type();
        let device_path = event.device().devnode()?;
        let device_info = match event_type {
            libudev::EventType::Add | libudev::EventType::Change => {
                if device_path.exists() {
                    identify_block_device(event)
                } else {
                    None
                }
            }
            _ => None,
        };

        if event_type == libudev::EventType::Add
            || (event_type == libudev::EventType::Change && device_info.is_some())
        {
            if let Some(info) = device_info {
                let stratis_identifiers = info.stratis_identifiers();
                let pool_uuid = stratis_identifiers.pool_uuid;
                let device_uuid = stratis_identifiers.device_uuid;
                if let Some((_, pool)) = pools.get_by_uuid(pool_uuid) {
                    match pool {
                        AnyPool::V1(p) => {
                            if p.get_strat_blockdev(device_uuid).is_none() {
                                warn!("Found a device with {info} that identifies itself as belonging to pool with UUID {pool_uuid}, but that pool is already up and running and does not appear to contain the device");
                            }
                        }
                        AnyPool::V2(p) => {
                            if p.get_strat_blockdev(device_uuid).is_none() {
                                warn!("Found a device with {info} that identifies itself as belonging to pool with UUID {pool_uuid}, but that pool is already up and running and does not appear to contain the device");
                            }
                        }
                    }
                    // FIXME: There might be something to check if the device is
                    // included in the pool, but that is less clear.
                    None
                } else {
                    let mut devices = self
                        .stopped_pools
                        .remove(&pool_uuid)
                        .or_else(|| self.partially_constructed_pools.remove(&pool_uuid))
                        .unwrap_or_default();

                    self.uuid_lookup
                        .insert(device_path.to_path_buf(), (pool_uuid, device_uuid));

                    devices.process_info_add(info);
                    match devices.pool_level_metadata_info() {
                        Ok((MaybeInconsistent::No(Some(name)), _)) => {
                            if let Some(maybe_conflict) = self.name_to_uuid.get_mut(&name) {
                                maybe_conflict.add(pool_uuid);
                                if let UuidOrConflict::Conflict(set) = maybe_conflict {
                                    warn!("Found conflicting names for stopped pools; UUID will be required to start pools with UUIDs {}", set.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(", "));
                                }
                            } else {
                                self.name_to_uuid
                                    .insert(name.clone(), UuidOrConflict::Uuid(pool_uuid));
                            }
                        }
                        Err(e) => {
                            info!("Error while attempting to determine pool name for pool with UUID {pool_uuid}: {e}; this may resolve when more devices appear and are processed");
                        }
                        _ => (),
                    }
                    self.try_setup_started_pool(pools, pool_uuid, devices)
                        .map(|(name, pool)| (name, pool_uuid, pool))
                }
            } else {
                None
            }
        } else if (event_type == libudev::EventType::Change && device_info.is_none())
            || event_type == libudev::EventType::Remove
        {
            let (pool_uuid, dev_uuid) =
                if let Some((pool_uuid, dev_uuid)) = self.uuid_lookup.get(device_path) {
                    (*pool_uuid, *dev_uuid)
                } else {
                    return None;
                };
            if self.stopped_pools.contains_key(&pool_uuid) {
                let mut devices = self.stopped_pools.remove(&pool_uuid).unwrap_or_default();

                devices.process_info_remove(device_path, pool_uuid, dev_uuid);
                self.uuid_lookup.remove(device_path);
                match devices.pool_level_metadata_info() {
                    Ok((MaybeInconsistent::No(Some(name)), _)) => {
                        if let Some(maybe_conflict) = self.name_to_uuid.get_mut(&name) {
                            maybe_conflict.add(pool_uuid);
                            if let UuidOrConflict::Conflict(set) = maybe_conflict {
                                warn!("Found conflicting names for stopped pools; UUID will be required to start pools with UUIDs {}", set.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(", "));
                            }
                        } else {
                            self.name_to_uuid
                                .insert(name, UuidOrConflict::Uuid(pool_uuid));
                        }
                    }
                    _ => {
                        self.name_to_uuid = self
                            .name_to_uuid
                            .drain()
                            .filter_map(|(n, mut maybe_conflict)| {
                                if maybe_conflict.remove(&pool_uuid) {
                                    None
                                } else {
                                    Some((n, maybe_conflict))
                                }
                            })
                            .collect();
                    }
                }

                if !devices.is_empty() {
                    self.stopped_pools.insert(pool_uuid, devices);
                }
            }
            None
        } else {
            None
        }
    }

    pub fn handle_stopped_pool(&mut self, pool_uuid: PoolUuid, device_set: DeviceSet) {
        if !device_set.is_empty() {
            match device_set.metadata_version() {
                Ok(mv) => {
                    match mv {
                        StratSigblockVersion::V1 => {
                            let dev_uuids = device_set
                                .iter()
                                .map(|(dev_uuid, _)| *dev_uuid)
                                .collect::<Vec<_>>();
                            if has_leftover_devices_legacy(pool_uuid, &dev_uuids) {
                                self.partially_constructed_pools
                                    .insert(pool_uuid, device_set);
                            } else {
                                self.stopped_pools.insert(pool_uuid, device_set);
                            }
                        }
                        StratSigblockVersion::V2 => {
                            if has_leftover_devices(pool_uuid) {
                                self.partially_constructed_pools
                                    .insert(pool_uuid, device_set);
                            } else {
                                self.stopped_pools.insert(pool_uuid, device_set);
                            }
                        }
                    };
                }
                Err(e) => {
                    warn!("Unable to detect leftover devices: {e}; putting in stopped pools");
                    self.stopped_pools.insert(pool_uuid, device_set);
                }
            }
        }
    }
}

impl Into<Value> for &LiminalDevices {
    fn into(self) -> Value {
        json!({
            "stopped_pools": Value::Array(
                self.stopped_pools
                    .iter()
                    .map(|(uuid, set)| {
                        json!({
                            "pool_uuid": uuid.to_string(),
                            "devices": <&DeviceSet as Into<Value>>::into(set),
                        })
                    })
                    .collect()
            ),
            "partially_constructed_pools": Value::Array(
                self.partially_constructed_pools
                    .iter()
                    .map(|(uuid, set)| {
                        json!({
                            "pool_uuid": uuid.to_string(),
                            "devices": <&DeviceSet as Into<Value>>::into(set),
                        })
                    })
                    .collect()
            ),
            "path_to_ids_map": Value::Object(
                self.uuid_lookup
                    .iter()
                    .map(|(path, (pool_uuid, dev_uuid))| {
                        (
                            path.display().to_string(),
                            Value::Array(vec![
                                Value::from(pool_uuid.to_string()),
                                Value::from(dev_uuid.to_string()),
                            ]),
                        )
                    })
                    .collect::<Map<_, _>>()
            ),
            "name_to_pool_uuid_map": Value::Object(
                self.name_to_uuid
                    .iter()
                    .map(|(name, maybe_conflict)| {
                        (
                            name.to_string(),
                            match maybe_conflict {
                                UuidOrConflict::Uuid(u) => Value::from(u.to_string()),
                                UuidOrConflict::Conflict(set) => Value::from(set.iter().map(|u| Value::from(u.to_string())).collect::<Vec<_>>())
                            },
                        )
                    })
                    .collect::<Map<_, _>>()
            )
        })
    }
}

/// Remove the cache from the metadata and return all device UUIDs corresponding to cache devices.
pub fn remove_cache_from_metadata(
    metadata: &mut PoolSave,
    infos: &HashMap<DevUuid, Box<LStratisInfo>>,
) -> Vec<(DevUuid, Option<Either<PathBuf, PathBuf>>)> {
    metadata
        .backstore
        .cache_tier
        .take()
        .map(|c| {
            c.blockdev
                .devs
                .iter()
                .map(|d| match infos.get(&d.uuid) {
                    Some(info) => (
                        d.uuid,
                        match &info.luks {
                            Some(l) => Some(Either::Right(l.dev_info.devnode.clone())),
                            None => Some(Either::Left(info.dev_info.devnode.clone())),
                        },
                    ),
                    None => (d.uuid, None),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Read the BDA and MDA information for a set of devices that has been
/// determined to be a part of the same pool.
///
/// If remove_cache is true, infos will be updated to remove the UUIDs of all cache devices.
fn load_stratis_metadata(
    pool_uuid: PoolUuid,
    infos: &mut HashMap<DevUuid, Box<LStratisInfo>>,
    remove_cache: bool,
) -> MetadataResult {
    if let Some((dev_uuid, info)) = infos.iter().find(|(dev_uuid, info)| {
        **dev_uuid != info.bda.dev_uuid() || pool_uuid != info.bda.pool_uuid()
    }) {
        return Err(
            StratisError::Msg(format!(
                "Mismatch between Stratis identifiers previously read and those found on some BDA: {} != {}",
                StratisIdentifiers::new(pool_uuid, *dev_uuid),
                StratisIdentifiers::new(info.bda.pool_uuid(), info.bda.dev_uuid())
            )));
    }

    match get_metadata(&stratis_infos_ref(infos)) {
        Ok(opt) => {
            let (timestamp, mut metadata) = opt
            .ok_or_else(|| {
                StratisError::Msg(format!(
                    "No metadata found on devices associated with pool UUID {pool_uuid}"
                ))
            })?;
            let dev_uuids_and_paths = remove_cache.then(|| {
                let dev_uuids_and_paths = remove_cache_from_metadata(&mut metadata, &*infos);
                for dev_uuid in dev_uuids_and_paths.iter().map(|(u, _)| u) {
                    infos.remove(dev_uuid);
                }
                dev_uuids_and_paths
            });
            Ok((timestamp, metadata, dev_uuids_and_paths.map(|v| v.into_iter().filter_map(|(_, path)| path).collect())))
        },
        Err(err) => Err(StratisError::Chained(
            format!(
                "There was an error encountered when reading the metadata for the devices found for pool with UUID {pool_uuid}"
            ),
            Box::new(err),
        ))
    }
}

/// Given a set of devices, try to set up a pool.
/// Return the pool information if a pool is set up. Otherwise, return
/// the pool information to the stopped pools data structure.
/// Do not attempt setup if the pool contains any unopened devices.
///
/// If there is a name conflict between the set of devices in devices
/// and some existing pool, return an error.
fn setup_pool_legacy(
    pools: &Table<PoolUuid, AnyPool>,
    pool_uuid: PoolUuid,
    luks_info: StratisResult<Option<PoolEncryptionInfo>>,
    infos: &HashMap<DevUuid, Box<LStratisInfo>>,
    timestamp: DateTime<Utc>,
    metadata: PoolSave,
    paths_to_wipe: Option<Vec<Either<PathBuf, PathBuf>>>,
) -> StratisResult<(Name, AnyPool)> {
    if let Some((uuid, _)) = pools.get_by_name(&metadata.name) {
        return Err(
            StratisError::Msg(format!(
                "There is a pool name conflict. The devices currently being processed have been identified as belonging to the pool with UUID {} and name {}, but a pool with the same name and UUID {} is already active",
                pool_uuid,
                &metadata.name,
                uuid
            ))
        );
    }

    let (datadevs, cachedevs) = match get_blockdevs_legacy(&metadata.backstore, infos) {
        Err(err) => return Err(
            StratisError::Chained(
                format!(
                    "There was an error encountered when calculating the block devices for pool with UUID {} and name {}",
                    pool_uuid,
                    &metadata.name,
                ),
                Box::new(err)
            )),
        Ok((datadevs, cachedevs)) => (datadevs, cachedevs),
    };

    let pool_einfo = match luks_info {
        Ok(inner) => inner,
        Err(_) => {
            // NOTE: This is not actually a hopeless situation. It may be
            // that a LUKS device owned by Stratis corresponding to a
            // Stratis device has just not been discovered yet. If it
            // is, the appropriate info will be updated, and setup may
            // yet succeed.
            return Err(
                StratisError::Msg(format!(
                        "Some data devices in the set belonging to pool with UUID {} and name {} appear to be encrypted devices managed by Stratis, and some do not",
                        pool_uuid,
                        &metadata.name
                ))
            );
        }
    };

    v1::StratPool::setup(pool_uuid, datadevs, cachedevs, timestamp, &metadata, paths_to_wipe, pool_einfo)
        .map(|(name, mut pool)| {
            if pool.blockdevs().iter().map(|(_, _, bd)| {
                bd.pool_name()
            }).any(|name| name != Some(Some(&Name::new(metadata.name.clone()))) || matches!(name, Some(None))) {
                if let Err(e) = pool.rename_pool(&name) {
                    warn!("Pool will not be able to be started by name; pool name metadata in LUKS2 token is not consistent across all devices: {e}");
                }
            }
            (name, AnyPool::V1(Box::new(pool)))
        })
        .map_err(|err| {
            StratisError::Chained(
                format!(
                    "An attempt to set up pool with UUID {pool_uuid} from the assembled devices failed"
                ),
                Box::new(err),
            )
        })
}

/// Given a set of devices, try to set up a pool.
/// Return the pool information if a pool is set up. Otherwise, return
/// the pool information to the stopped pools data structure.
/// Do not attempt setup if the pool contains any unopened devices.
///
/// If there is a name conflict between the set of devices in devices
/// and some existing pool, return an error.
#[allow(clippy::too_many_arguments)]
fn setup_pool(
    pools: &Table<PoolUuid, AnyPool>,
    pool_uuid: PoolUuid,
    infos: &HashMap<DevUuid, Box<LStratisInfo>>,
    timestamp: DateTime<Utc>,
    metadata: PoolSave,
    paths_to_wipe: Option<Vec<PathBuf>>,
    token_slot: TokenUnlockMethod,
    passphrase: Option<SizedKeyMemory>,
) -> StratisResult<(Name, AnyPool)> {
    if let Some((uuid, _)) = pools.get_by_name(&metadata.name) {
        return Err(
                StratisError::Msg(format!(
                    "There is a pool name conflict. The devices currently being processed have been identified as belonging to the pool with UUID {} and name {}, but a pool with the same name and UUID {} is already active",
                    pool_uuid,
                    &metadata.name,
                    uuid
                )));
    }

    let (datadevs, cachedevs) = match get_blockdevs(&metadata.backstore, infos) {
            Err(err) => return Err(
                StratisError::Chained(
                    format!(
                        "There was an error encountered when calculating the block devices for pool with UUID {} and name {}",
                        pool_uuid,
                        &metadata.name,
                    ),
                    Box::new(err)
                )),
            Ok((datadevs, cachedevs)) => (datadevs, cachedevs),
        };

    let dev = datadevs.first();
    if dev.is_none() {
        return Err(StratisError::Msg(format!(
            "There do not appear to be any data devices in the set with pool UUID {pool_uuid}"
        )));
    }

    v2::StratPool::setup(
        pool_uuid,
        datadevs,
        cachedevs,
        timestamp,
        &metadata,
        paths_to_wipe,
        token_slot,
        passphrase,
    )
    .map(|(name, pool)| (name, AnyPool::V2(Box::new(pool))))
    .map_err(|err| {
        StratisError::Chained(
            format!(
                "An attempt to set up pool with UUID {pool_uuid} from the assembled devices failed"
            ),
            Box::new(err),
        )
    })
}

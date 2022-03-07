// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Management of devices which are known to stratisd but not in a pool.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::{
    engine::{
        engine::Pool,
        strat_engine::{
            backstore::{find_stratis_devs_by_uuid, CryptActivationHandle, CryptHandle},
            liminal::{
                device_info::{DeviceSet, LInfo, LLuksInfo, LStratisInfo},
                identify::{identify_block_device, DeviceInfo, LuksInfo, StratisInfo},
                setup::{get_bdas, get_blockdevs, get_metadata, get_pool_state},
            },
            metadata::{StratisIdentifiers, BDA},
            pool::StratPool,
            serde_structs::PoolSave,
        },
        structures::Table,
        types::{
            DevUuid, LockedPoolInfo, Name, PoolUuid, StoppedPoolInfo, UdevEngineEvent, UnlockMethod,
        },
    },
    stratis::{StratisError, StratisResult},
};

/// Devices which stratisd has discovered but which have not been assembled
/// into pools.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct LiminalDevices {
    /// Lookup data structure for pool and device UUIDs corresponding with
    /// a path where the superblock was either removed or the device was removed.
    uuid_lookup: HashMap<PathBuf, (PoolUuid, DevUuid)>,
    /// Devices that have not yet been set up or have been stopped.
    stopped_pools: HashMap<PoolUuid, DeviceSet>,
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

    /// Unlock the liminal encrypted devices that correspond to the given pool UUID.
    pub fn unlock_pool(
        &mut self,
        pools: &Table<PoolUuid, StratPool>,
        pool_uuid: PoolUuid,
        unlock_method: UnlockMethod,
    ) -> StratisResult<Vec<(DevUuid, CryptHandle)>> {
        fn handle_luks(
            luks_info: &LLuksInfo,
            unlock_method: UnlockMethod,
        ) -> StratisResult<CryptHandle> {
            if let Some(h) = CryptActivationHandle::setup(&luks_info.ids.devnode, unlock_method)? {
                Ok(h)
            } else {
                Err(StratisError::Msg(format!(
                    "Block device {} does not appear to be formatted with
                        the proper Stratis LUKS2 metadata.",
                    luks_info.ids.devnode.display(),
                )))
            }
        }

        let unlocked = match self.stopped_pools.get(&pool_uuid) {
            Some(map) => {
                let encryption_info = map.encryption_info();
                if let Ok(None) = encryption_info {
                    return Err(StratisError::Msg(
                        format!(
                            "Attempted to unlock set of devices belonging to an unencrypted pool with UUID {}",
                            pool_uuid,
                        ),
                    ));
                } else if let Err(e) = encryption_info {
                    return Err(StratisError::Chained(
                        format!(
                            "Error in the encryption information for pool with UUID {}",
                            pool_uuid,
                        ),
                        Box::new(e),
                    ));
                }

                let mut unlocked = Vec::new();
                for (dev_uuid, info) in map.iter() {
                    match info {
                        LInfo::Stratis(_) => (),
                        LInfo::Luks(ref luks_info) => match handle_luks(luks_info, unlock_method) {
                            Ok(handle) => unlocked.push((*dev_uuid, handle)),
                            Err(e) => {
                                return Err(handle_unlock_rollback(
                                    e,
                                    unlocked
                                        .into_iter()
                                        .map(|(_, handle)| handle)
                                        .collect::<Vec<_>>(),
                                ));
                            }
                        },
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
                            "Pool with UUID {} is not encrypted and cannot be unlocked.",
                            pool_uuid,
                        )));
                    }
                }
                None => {
                    return Err(StratisError::Msg(format!(
                        "No devices with UUID {} have been registered with stratisd.",
                        pool_uuid,
                    )))
                }
            },
        };

        Ok(unlocked)
    }

    /// Start a pool, create the devicemapper devices, and return the fully constructed
    /// pool.
    pub fn start_pool(
        &mut self,
        pools: &Table<PoolUuid, StratPool>,
        pool_uuid: PoolUuid,
        unlock_method: Option<UnlockMethod>,
    ) -> StratisResult<(Name, StratPool)> {
        let encryption_info = self
            .stopped_pools
            .get(&pool_uuid)
            .ok_or_else(|| {
                StratisError::Msg(format!(
                    "Requested pool with UUID {} was not found in stopped pools",
                    pool_uuid
                ))
            })?
            .encryption_info();
        let unlocked_devices = match (encryption_info, unlock_method) {
            (Ok(Some(_)), None) => {
                return Err(StratisError::Msg(format!(
                    "Pool with UUID {} is encrypted but no unlock method was provided",
                    pool_uuid,
                )));
            }
            (Ok(None), None) => Vec::new(),
            (Ok(Some(_)), Some(method)) => self.unlock_pool(pools, pool_uuid, method)?,
            (Ok(None), Some(_)) => {
                return Err(StratisError::Msg(format!(
                    "Pool with UUID {} is not encrypted but an unlock method was provided",
                    pool_uuid,
                )));
            }
            (Err(e), _) => return Err(e),
        };

        let mut stopped_pool = self
            .stopped_pools
            .remove(&pool_uuid)
            .expect("Checked above");
        match find_stratis_devs_by_uuid(
            pool_uuid,
            unlocked_devices
                .iter()
                .map(|(dev_uuid, _)| *dev_uuid)
                .collect::<Vec<_>>(),
        ) {
            Ok(infos) => {
                for info in infos.into_iter().map(|(dev_uuid, (path, devno))| {
                    self.uuid_lookup
                        .insert(path.to_path_buf(), (pool_uuid, dev_uuid));
                    DeviceInfo::Stratis(StratisInfo {
                        device_number: devno,
                        devnode: path.to_path_buf(),
                        identifiers: StratisIdentifiers {
                            pool_uuid,
                            device_uuid: dev_uuid,
                        },
                    })
                }) {
                    stopped_pool.process_info_add(info);
                }
            }
            Err(e) => {
                warn!("Failed to scan for newly unlocked Stratis devices: {}", e);
            }
        }

        match self.try_setup_pool(pools, pool_uuid, stopped_pool) {
            Ok((name, pool)) => Ok((name, pool)),
            Err(e) => Err(handle_unlock_rollback(
                e,
                unlocked_devices
                    .into_iter()
                    .map(|(_, h)| h)
                    .collect::<Vec<_>>(),
            )),
        }
    }

    /// Stop a pool, tear down the devicemapper devices, and store the pool information
    /// in an internal data structure for later starting.
    pub fn stop_pool(
        &mut self,
        pool_name: &Name,
        pool_uuid: PoolUuid,
        pool: &mut StratPool,
    ) -> StratisResult<()> {
        let devices = pool.stop(pool_name)?;
        self.stopped_pools.insert(pool_uuid, devices);
        Ok(())
    }

    /// Get a mapping of pool UUIDs from all of the LUKS2 devices that are currently
    /// locked to their encryption info in the set of pools that are not yet set up.
    pub fn locked_pools(&self) -> HashMap<PoolUuid, LockedPoolInfo> {
        self.stopped_pools
            .iter()
            .filter_map(|(pool_uuid, map)| map.locked_pool_info().map(|info| (*pool_uuid, info)))
            .collect()
    }

    /// Get a mapping of pool UUIDs to device sets for all stopped pools.
    pub fn stopped_pools(&self) -> HashMap<PoolUuid, StoppedPoolInfo> {
        self.stopped_pools
            .iter()
            .filter_map(|(pool_uuid, map)| map.stopped_pool_info().map(|info| (*pool_uuid, info)))
            .collect()
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
    ) -> Vec<(Name, PoolUuid, StratPool)> {
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
                let mut infos: Vec<DeviceInfo> = stratis_infos
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

                for info in infos.iter() {
                    let linfo = LInfo::from(info.clone());
                    match linfo {
                        LInfo::Luks(l) => {
                            self.uuid_lookup.insert(
                                l.ids.devnode.clone(),
                                (l.ids.identifiers.pool_uuid, l.ids.identifiers.device_uuid),
                            );
                        }
                        LInfo::Stratis(s) => {
                            if let Some(l) = s.luks.as_ref() {
                                self.uuid_lookup.insert(
                                    l.ids.devnode.clone(),
                                    (l.ids.identifiers.pool_uuid, l.ids.identifiers.device_uuid),
                                );
                            }
                            self.uuid_lookup.insert(
                                s.ids.devnode.clone(),
                                (s.ids.identifiers.pool_uuid, s.ids.identifiers.device_uuid),
                            );
                        }
                    }
                }

                let mut info_map = DeviceSet::new();
                while !infos.is_empty() {
                    let info: DeviceInfo = infos.pop().expect("!infos.is_empty()");
                    info_map.process_info_add(info);
                }

                self.try_setup_started_pool(&table, *pool_uuid, info_map)
                    .map(|(pool_name, pool)| (pool_name, *pool_uuid, pool))
            })
            .collect::<Vec<(Name, PoolUuid, StratPool)>>()
    }

    /// Attempt to set up a pool, starting it if it is not already started.
    ///
    /// See documentation for setup_pool for more information.
    ///
    /// Precondition: pools.get_by_uuid(pool_uuid).is_none() &&
    ///               self.stopped_pools.get(pool_uuid).is_none()
    fn try_setup_pool(
        &mut self,
        pools: &Table<PoolUuid, StratPool>,
        pool_uuid: PoolUuid,
        device_set: DeviceSet,
    ) -> StratisResult<(Name, StratPool)> {
        fn try_setup_pool_failure(
            pools: &Table<PoolUuid, StratPool>,
            pool_uuid: PoolUuid,
            device_set: &DeviceSet,
        ) -> StratisResult<(Name, StratPool)> {
            let infos = match device_set.as_opened_set() {
                Some(i) => i,
                None => {
                    return Err(StratisError::Msg(format!(
                        "Some of the devices in pool with UUID {} are unopened",
                        pool_uuid,
                    )))
                }
            };
            let (bdas, timestamp, metadata) = load_stratis_metadata(pool_uuid, &infos)?;
            setup_pool(
                pools, pool_uuid, device_set, &infos, bdas, timestamp, metadata,
            )
        }

        assert!(pools.get_by_uuid(pool_uuid).is_none());
        assert!(self.stopped_pools.get(&pool_uuid).is_none());

        match try_setup_pool_failure(pools, pool_uuid, &device_set) {
            Ok((name, pool)) => {
                self.uuid_lookup = self
                    .uuid_lookup
                    .drain()
                    .filter(|(_, (p, _))| *p != pool_uuid)
                    .collect();
                info!(
                    "Pool with name \"{}\" and UUID \"{}\" set up",
                    name, pool_uuid
                );
                Ok((name, pool))
            }
            Err(err) => {
                info!("Attempt to set up pool failed, but it may be possible to set up the pool later, if the situation changes: {}", err);
                if !device_set.is_empty() {
                    self.stopped_pools.insert(pool_uuid, device_set);
                }
                Err(err)
            }
        }
    }

    /// Variation on try_setup_pool that returns None if the pool is marked
    /// as stopped in its metadata.
    ///
    /// Precondition: pools.get_by_uuid(pool_uuid).is_none() &&
    ///               self.stopped_pools.get(pool_uuid).is_none()
    fn try_setup_started_pool(
        &mut self,
        pools: &Table<PoolUuid, StratPool>,
        pool_uuid: PoolUuid,
        device_set: DeviceSet,
    ) -> Option<(Name, StratPool)> {
        fn try_setup_started_pool_failure(
            pools: &Table<PoolUuid, StratPool>,
            pool_uuid: PoolUuid,
            device_set: &DeviceSet,
        ) -> StratisResult<Option<(Name, StratPool)>> {
            let infos = match device_set.as_opened_set() {
                Some(i) => i,
                None => {
                    return Err(StratisError::Msg(format!(
                        "Some of the devices in pool with UUID {} are unopened",
                        pool_uuid,
                    )))
                }
            };
            let (bdas, timestamp, metadata) = load_stratis_metadata(pool_uuid, &infos)?;
            if let Some(true) | None = metadata.started {
                setup_pool(
                    pools, pool_uuid, device_set, &infos, bdas, timestamp, metadata,
                )
                .map(Some)
            } else {
                Ok(None)
            }
        }

        assert!(pools.get_by_uuid(pool_uuid).is_none());
        assert!(self.stopped_pools.get(&pool_uuid).is_none());

        match try_setup_started_pool_failure(pools, pool_uuid, &device_set) {
            Ok(Some((name, pool))) => {
                self.uuid_lookup = self
                    .uuid_lookup
                    .drain()
                    .filter(|(_, (p, _))| *p != pool_uuid)
                    .collect();
                info!(
                    "Pool with name \"{}\" and UUID \"{}\" set up",
                    name, pool_uuid
                );
                Some((name, pool))
            }
            Ok(None) => {
                if !device_set.is_empty() {
                    self.stopped_pools.insert(pool_uuid, device_set);
                }
                None
            }
            Err(err) => {
                info!("Attempt to set up pool failed, but it may be possible to set up the pool later, if the situation changes: {}", err);
                if !device_set.is_empty() {
                    self.stopped_pools.insert(pool_uuid, device_set);
                }
                None
            }
        }
    }

    /// Given some information gathered about a single Stratis device, determine
    /// whether or not a pool can be constructed, and if it can, construct the
    /// pool and return the newly constructed pool. If the device appears to
    /// belong to a pool that has already been set up assume that no further
    /// processing is required and return None. If there is an error
    /// constructing the pool, retain the set of devices.
    pub fn block_evaluate(
        &mut self,
        pools: &Table<PoolUuid, StratPool>,
        event: &UdevEngineEvent,
    ) -> Option<(Name, PoolUuid, StratPool)> {
        let event_type = event.event_type();
        let device_path = match event.device().devnode() {
            Some(d) => d,
            None => return None,
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

        if event_type == libudev::EventType::Add
            || (event_type == libudev::EventType::Change && device_info.is_some())
        {
            if let Some(info) = device_info {
                let stratis_identifiers = info.stratis_identifiers();
                let pool_uuid = stratis_identifiers.pool_uuid;
                let device_uuid = stratis_identifiers.device_uuid;
                if let Some((_, pool)) = pools.get_by_uuid(pool_uuid) {
                    if pool.get_strat_blockdev(device_uuid).is_none() {
                        warn!("Found a device with {} that identifies itself as belonging to pool with UUID {}, but that pool is already up and running and does not appear to contain the device",
                              info,
                              pool_uuid);
                    }
                    // FIXME: There might be something to check if the device is
                    // included in the pool, but that is less clear.
                    None
                } else {
                    let mut devices = self
                        .stopped_pools
                        .remove(&pool_uuid)
                        .unwrap_or_else(DeviceSet::new);

                    self.uuid_lookup
                        .insert(device_path.to_path_buf(), (pool_uuid, device_uuid));

                    devices.process_info_add(info);
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
            if pools
                .get_by_uuid(pool_uuid)
                .and_then(|(_, p)| p.get_strat_blockdev(dev_uuid))
                .is_some()
            {
                warn!("udev reports that a device with UUID {} that appears to belong to a pool with UUID {} has just been removed; this is likely to result in data loss",
                      dev_uuid,
                      pool_uuid);
            } else if self.stopped_pools.get(&pool_uuid).is_some() {
                let mut devices = self
                    .stopped_pools
                    .remove(&pool_uuid)
                    .unwrap_or_else(DeviceSet::new);

                devices.process_info_remove(device_path, pool_uuid, dev_uuid);
                self.uuid_lookup.remove(device_path);
                if !devices.is_empty() {
                    self.stopped_pools.insert(pool_uuid, devices);
                }
            }
            None
        } else {
            None
        }
    }
}

impl<'a> Into<Value> for &'a LiminalDevices {
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
            )
        })
    }
}

/// Read the BDA and MDA information for a set of devices that has been
/// determined to be a part of the same pool.
fn load_stratis_metadata(
    pool_uuid: PoolUuid,
    infos: &HashMap<DevUuid, &LStratisInfo>,
) -> StratisResult<(HashMap<DevUuid, BDA>, DateTime<Utc>, PoolSave)> {
    let bdas = match get_bdas(infos) {
        Err(err) => Err(StratisError::Chained(
            format!(
                "There was an error encountered when reading the BDAs for the devices found for pool with UUID {}",
                pool_uuid,
            ),
            Box::new(err)
        )),
        Ok(infos) => Ok(infos),
    }?;

    if let Some((dev_uuid, bda)) = bdas
        .iter()
        .find(|(dev_uuid, bda)| **dev_uuid != bda.dev_uuid() || pool_uuid != bda.pool_uuid())
    {
        return Err(
            StratisError::Msg(format!(
                "Mismatch between Stratis identifiers previously read and those found on some BDA: {} != {}",
                StratisIdentifiers::new(pool_uuid, *dev_uuid),
                StratisIdentifiers::new(bda.pool_uuid(), bda.dev_uuid())
            )));
    }

    match get_metadata(infos, &bdas) {
        Err(err) => Err(
            StratisError::Chained(
                format!(
                    "There was an error encountered when reading the metadata for the devices found for pool with UUID {}",
                    pool_uuid,
                ),
                Box::new(err)
            )),
        Ok(None) => Err(StratisError::Msg(format!(
            "No metadata found on devices associated with pool UUID {}",
            pool_uuid
        ))),
        Ok(Some((timestamp, metadata))) => Ok((bdas, timestamp, metadata)),
    }
}

/// Given a set of devices, try to set up a pool.
/// Return the pool information if a pool is set up. Otherwise, return
/// the pool information to the stopped pools data structure.
/// Do not attempt setup if the pool contains any unopened devices.
///
/// If there is a name conflict between the set of devices in devices
/// and some existing pool, return an error.
fn setup_pool(
    pools: &Table<PoolUuid, StratPool>,
    pool_uuid: PoolUuid,
    device_set: &DeviceSet,
    infos: &HashMap<DevUuid, &LStratisInfo>,
    bdas: HashMap<DevUuid, BDA>,
    timestamp: DateTime<Utc>,
    metadata: PoolSave,
) -> StratisResult<(Name, StratPool)> {
    if let Some((uuid, _)) = pools.get_by_name(&metadata.name) {
        return Err(
            StratisError::Msg(format!(
                "There is a pool name conflict. The devices currently being processed have been identified as belonging to the pool with UUID {} and name {}, but a pool with the same name and UUID {} is already active",
                pool_uuid,
                &metadata.name,
                uuid
            )));
    }

    let (datadevs, cachedevs) = match get_blockdevs(&metadata.backstore, infos, bdas) {
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

    if datadevs.get(0).is_none() {
        return Err(StratisError::Msg(format!(
            "There do not appear to be any data devices in the set with pool UUID {}",
            pool_uuid
        )));
    }

    let encryption_info = match device_set.encryption_info() {
        Ok(opt) => opt,
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
                )));
        }
    };

    let state = get_pool_state(encryption_info);
    StratPool::setup(pool_uuid, datadevs, cachedevs, timestamp, &metadata, state).map_err(|err| {
        StratisError::Chained(
            format!(
                "An attempt to set up pool with UUID {} from the assembled devices failed",
                pool_uuid
            ),
            Box::new(err),
        )
    })
}

/// Rollback an unlock operation for some or all devices of a pool that have been
/// unlocked prior to the failure occuring.
fn handle_unlock_rollback(causal_error: StratisError, handles: Vec<CryptHandle>) -> StratisError {
    for handle in handles {
        if let Err(e) = handle.deactivate() {
            warn!("Failed to roll back encrypted pool unlock; some previously locked encrypted devices may be left in an unlocked state");
            return StratisError::NoActionRollbackError {
                causal_error: Box::new(causal_error),
                rollback_error: Box::new(StratisError::Chained(
                    "Failed to roll back encrypted pool unlock; some previously locked encrypted devices may be left in an unlocked state".to_string(),
                    Box::new(e),
                )),
            };
        }
    }

    causal_error
}

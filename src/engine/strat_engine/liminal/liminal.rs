// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Management of devices which are known to stratisd but not in a pool.

use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use serde_json::Value;

use crate::{
    engine::{
        engine::Pool,
        strat_engine::{
            backstore::CryptHandle,
            liminal::{
                device_info::{DeviceBag, DeviceSet, LInfo, LLuksInfo, LStratisInfo},
                identify::{identify_block_device, DeviceInfo, LuksInfo, StratisInfo},
                setup::{get_bdas, get_blockdevs, get_metadata},
            },
            metadata::StratisIdentifiers,
            pool::StratPool,
        },
        structures::Table,
        types::{DevUuid, KeyDescription, Name, PoolUuid, UnlockMethod},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

/// On an error, whether this set of devices is hopeless or just errored
#[derive(Debug)]
enum Destination {
    Hopeless(String),
    Errored(String),
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Destination::Hopeless(val) => write!(f, "{}", val),
            Destination::Errored(val) => write!(f, "{}", val),
        }
    }
}

/// Devices which stratisd has discovered but which have not been assembled
/// into pools.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct LiminalDevices {
    /// Sets of devices which have not been promoted to pools, but which
    /// may still have a chance.
    errored_pool_devices: HashMap<PoolUuid, DeviceSet>,
    /// Sets of devices which possess some internal contradiction which makes
    /// it impossible for them to be made into sensible pools ever.
    hopeless_device_sets: HashMap<PoolUuid, DeviceBag>,
}

impl LiminalDevices {
    #[allow(dead_code)]
    fn invariant(&self) {
        assert!(self
            .errored_pool_devices
            .keys()
            .cloned()
            .collect::<HashSet<PoolUuid>>()
            .intersection(
                &self
                    .hopeless_device_sets
                    .keys()
                    .cloned()
                    .collect::<HashSet<PoolUuid>>()
            )
            .next()
            .is_none());
    }

    /// Unlock the liminal encrypted devices that correspond to the given pool UUID.
    pub fn unlock_pool(
        &mut self,
        pools: &Table<StratPool>,
        pool_uuid: PoolUuid,
        unlock_method: UnlockMethod,
    ) -> StratisResult<Vec<DevUuid>> {
        fn handle_luks(luks_info: &LLuksInfo, unlock_method: UnlockMethod) -> StratisResult<()> {
            if let Some(mut handle) = CryptHandle::setup(&luks_info.ids.devnode)? {
                handle.activate(unlock_method)?;
                Ok(())
            } else {
                Err(StratisError::Engine(
                    ErrorEnum::Invalid,
                    format!(
                        "Block device {} does not appear to be formatted with
                        the proper Stratis LUKS2 metadata.",
                        luks_info.ids.devnode.display(),
                    ),
                ))
            }
        }

        let unlocked = match self.errored_pool_devices.get(&pool_uuid) {
            Some(map) => {
                if map.all_unencrypted() {
                    return Err(StratisError::Engine(
                        ErrorEnum::Error,
                        format!(
                            "Attempted to unlock set of devices belonging to an unencrypted pool with UUID {}",
                            pool_uuid.to_simple_ref(),
                        ),
                    ));
                }

                let mut unlocked = Vec::new();
                for (dev_uuid, info) in map.iter() {
                    match info {
                        LInfo::Stratis(_) => (),
                        LInfo::Luks(ref luks_info) => match handle_luks(luks_info, unlock_method) {
                            Ok(()) => unlocked.push(*dev_uuid),
                            Err(e) => return Err(e),
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
                        return Err(StratisError::Engine(
                            ErrorEnum::Error,
                            format!(
                                "Pool with UUID {} is not encrypted and cannot be unlocked.",
                                pool_uuid.to_simple_ref()
                            ),
                        ));
                    }
                }
                None => {
                    return Err(StratisError::Engine(
                        ErrorEnum::Error,
                        format!(
                            "No devices with UUID {} have been registered with stratisd.",
                            pool_uuid.to_simple_ref(),
                        ),
                    ))
                }
            },
        };

        Ok(unlocked)
    }

    /// Get a mapping of pool UUIDs from all of the LUKS2 devices that are currently
    /// locked to their key descriptions in the set of pools that are not yet set up.
    // Precondition: All devices for a given errored pool have been determined to have
    // the same key description.
    pub fn locked_pools(&self) -> HashMap<PoolUuid, KeyDescription> {
        self.errored_pool_devices
            .iter()
            .filter_map(|(pool_uuid, map)| map.key_description().map(|kd| (*pool_uuid, kd.clone())))
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
                    .unwrap_or_else(Vec::new)
                    .drain(..)
                    .map(DeviceInfo::Stratis)
                    .chain(
                        luks_infos
                            .unwrap_or_else(Vec::new)
                            .drain(..)
                            .map(DeviceInfo::Luks),
                    )
                    .collect();

                let mut info_map = DeviceSet::new();
                while !infos.is_empty() && !self.hopeless_device_sets.contains_key(pool_uuid) {
                    let info: DeviceInfo = infos.pop().expect("!infos.is_empty()");
                    if let Err(mut hopeless) = info_map.process_info_add(info) {
                        hopeless.extend(infos.drain(..).map(|x| x.into()));
                        self.hopeless_device_sets.insert(*pool_uuid, hopeless);
                    }
                }

                if !self.hopeless_device_sets.contains_key(pool_uuid) {
                    self.try_setup_pool(&table, *pool_uuid, info_map)
                        .map(|(pool_name, pool)| (pool_name, *pool_uuid, pool))
                } else {
                    None
                }
            })
            .collect::<Vec<(Name, PoolUuid, StratPool)>>()
    }

    /// Given a set of devices, try to set up a pool.
    /// Return the pool information if a pool is set up. Otherwise, distribute
    /// the pool information to the appropriate data structure.
    /// Do not attempt setup if the pool contains any unopened devices.
    ///
    /// If there is a name conflict between the set of devices in devices
    /// and some existing pool, return an error.
    ///
    /// Precondition: pools.get_by_uuid(pool_uuid).is_none() &&
    ///               self.errored_pool_devices.get(pool_uuid).is_none() &&
    ///               self.hopeless_device_sets.get(pool_uuid).is_none()
    fn try_setup_pool(
        &mut self,
        pools: &Table<StratPool>,
        pool_uuid: PoolUuid,
        infos: DeviceSet,
    ) -> Option<(Name, StratPool)> {
        assert!(pools.get_by_uuid(pool_uuid).is_none());
        assert!(self.errored_pool_devices.get(&pool_uuid).is_none());
        assert!(self.hopeless_device_sets.get(&pool_uuid).is_none());

        // Setup a pool from constituent devices in the context of some already
        // setup pools.
        //
        // Precondition: every device represented by an item in infos has
        // already been determined to belong to the pool with pool_uuid.
        fn setup_pool(
            pools: &Table<StratPool>,
            pool_uuid: PoolUuid,
            infos: &HashMap<DevUuid, &LStratisInfo>,
        ) -> Result<(Name, StratPool), Destination> {
            let bdas = match get_bdas(infos) {
                Err(err) => Err(
                    Destination::Errored(format!(
                        "There was an error encountered when reading the BDAs for the devices found for pool with UUID {}: {}",
                        pool_uuid.to_simple_ref(),
                        err))),
                Ok(infos) => Ok(infos),
            }?;

            if let Some((dev_uuid, bda)) = bdas.iter().find(|(dev_uuid, bda)| {
                **dev_uuid != bda.dev_uuid() || pool_uuid != bda.pool_uuid()
            }) {
                return Err(
                    Destination::Hopeless(format!(
                        "Mismatch between Stratis identifiers previously read and those found on some BDA: {} != {}",
                        StratisIdentifiers::new(pool_uuid, *dev_uuid),
                        StratisIdentifiers::new(bda.pool_uuid(), bda.dev_uuid())
                        )));
            }

            let (timestamp, metadata) = match get_metadata(infos, &bdas) {
                Err(err) => return Err(
                    Destination::Errored(format!(
                        "There was an error encountered when reading the metadata for the devices found for pool with UUID {}: {}",
                        pool_uuid.to_simple_ref(),
                        err))),
                Ok(None) => return Err(
                    Destination::Errored(format!(
                        "No metadata found on devices associated with pool UUID {}",
                        pool_uuid.to_simple_ref()))),
                Ok(Some((timestamp, metadata))) => (timestamp, metadata),
            };

            if let Some((uuid, _)) = pools.get_by_name(&metadata.name) {
                return Err(
                    Destination::Errored(format!(
                        "There is a pool name conflict. The devices currently being processed have been identified as belonging to the pool with UUID {} and name {}, but a pool with the same name and UUID {} is already active",
                        pool_uuid.to_simple_ref(),
                        &metadata.name,
                        uuid.to_simple_ref())));
            }

            let (datadevs, cachedevs) = match get_blockdevs(&metadata.backstore, infos, bdas) {
                Err(err) => return Err(
                    Destination::Errored(format!(
                        "There was an error encountered when calculating the block devices for pool with UUID {} and name {}: {}",
                        pool_uuid.to_simple_ref(),
                        &metadata.name,
                        err))),
                Ok((datadevs, cachedevs)) => (datadevs, cachedevs),
            };

            if datadevs.get(0).is_none() {
                return Err(Destination::Hopeless(format!(
                    "There do not appear to be any data devices in the set with pool UUID {}",
                    pool_uuid.to_simple_ref()
                )));
            }

            // NOTE: DeviceSet provides infos variable in setup_pool. DeviceSet
            // ensures that all key descriptions match so we do not need to
            // check again here.
            let num_with_luks = datadevs
                .iter()
                .filter_map(|sbd| sbd.key_description())
                .count();

            if num_with_luks != 0 && num_with_luks != datadevs.len() {
                // NOTE: This is not actually a hopeless situation. It may be
                // that a LUKS device owned by Stratis corresponding to a
                // Stratis device has just not been discovered yet. If it
                // is, the appropriate info will be updated, and setup may
                // yet succeed.
                return Err(
                    Destination::Errored(format!(
                            "Some data devices in the set belonging to pool with UUID {} and name {} appear to be encrypted devices managed by Stratis, and some do not",
                            pool_uuid.to_simple_ref(),
                            &metadata.name)));
            }

            StratPool::setup(pool_uuid, datadevs, cachedevs, timestamp, &metadata).map_err(|err| {
                Destination::Errored(format!(
                    "An attempt to set up pool with UUID {} from the assembled devices failed: {}",
                    pool_uuid.to_simple_ref(),
                    err
                ))
            })
        }

        let opened = match infos.as_opened_set() {
            Some(opened) => opened,
            None => {
                self.errored_pool_devices.insert(pool_uuid, infos);
                return None;
            }
        };

        let result = setup_pool(pools, pool_uuid, &opened);

        match result {
            Ok((pool_name, pool)) => {
                info!(
                    "Pool with name \"{}\" and UUID \"{}\" set up",
                    pool_name,
                    pool_uuid.to_simple_ref()
                );
                Some((pool_name, pool))
            }
            Err(Destination::Hopeless(err)) => {
                warn!(
                    "Attempt to set up pool failed, moving to hopeless devices: {}",
                    err
                );
                self.hopeless_device_sets
                    .insert(pool_uuid, infos.into_bag());
                None
            }
            Err(Destination::Errored(err)) => {
                info!("Attempt to set up pool failed, but it may be possible to set up the pool later, if the situation changes: {}", err);
                self.errored_pool_devices.insert(pool_uuid, infos);
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
        pools: &Table<StratPool>,
        event: &libudev::Event,
    ) -> Option<(PoolUuid, Name, StratPool)> {
        let event_type = event.event_type();
        if event_type == libudev::EventType::Add || event_type == libudev::EventType::Change {
            identify_block_device(event.device()).and_then(move |info| {
                let stratis_identifiers = info.stratis_identifiers();
                let pool_uuid = stratis_identifiers.pool_uuid;
                let device_uuid = stratis_identifiers.device_uuid;
                if let Some((_, pool)) = pools.get_by_uuid(pool_uuid) {
                    if pool.get_strat_blockdev(device_uuid).is_none() {
                        warn!("Found a device with {} that identifies itself as belonging to pool with UUID {}, but that pool is already up and running and does not appear to contain the device",
                              info,
                              pool_uuid.to_simple_ref());
                    }
                    // FIXME: There might be something to check if the device is
                    // included in the pool, but that is less clear.
                    None
                } else if let Some(mut set) = self.hopeless_device_sets.remove(&pool_uuid) {
                    set.insert(info.into());
                    self.hopeless_device_sets.insert(pool_uuid, set);
                    None
                } else {
                    let mut devices = self
                        .errored_pool_devices
                        .remove(&pool_uuid)
                        .unwrap_or_else(DeviceSet::new);

                    if let Err(hopeless) = devices.process_info_add(info) {
                        self.hopeless_device_sets.insert(pool_uuid, hopeless);
                        return None;
                    }

                    // FIXME: An attempt to set up the pool is made, even if no
                    // new device has been added to the set of devices that appear
                    // to belong to the pool. The reason for this is that there
                    // may be many causes of failure to set up a pool, and that
                    // it may be worth another try. If an attempt to setup the
                    // pool is only made on discovery of a new device that may
                    // leave a pool that could be set up in limbo forever. An
                    // alternative, where the user can explicitly ask to try to
                    // set up an incomplete pool would be a better choice.
                    self.try_setup_pool(pools, pool_uuid, devices)
                        .map(|(name, pool)| (pool_uuid, name, pool))
                }
            })
        } else if event_type == libudev::EventType::Remove {
            identify_block_device(event.device()).and_then(move |info| {
                let stratis_identifiers = info.stratis_identifiers();
                let pool_uuid = stratis_identifiers.pool_uuid;
                let device_uuid = stratis_identifiers.device_uuid;
                if let Some((_, pool)) = pools.get_by_uuid(pool_uuid) {
                    if pool.get_strat_blockdev(device_uuid).is_some() {
                        warn!("udev reports that a device with {} that appears to belong to a pool with UUID {} has just been removed; this is likely to result in data loss",
                              info,
                              pool_uuid.to_simple_ref());
                    }
                    None
                } else if let Some(mut set) = self.hopeless_device_sets.remove(&pool_uuid) {
                    set.remove(&info.into());
                    self.hopeless_device_sets.insert(pool_uuid, set);
                    None
                } else {
                    let mut devices = self
                        .errored_pool_devices
                        .remove(&pool_uuid)
                        .unwrap_or_else(DeviceSet::new);

                    devices.process_info_remove(info);

                    self.try_setup_pool(pools, pool_uuid, devices)
                        .map(|(name, pool)| (pool_uuid, name, pool))
                }
            })
        } else {
            None
        }
    }
}

impl<'a> Into<Value> for &'a LiminalDevices {
    fn into(self) -> Value {
        json!({
            "errored_pools": Value::Array(
                self.errored_pool_devices
                    .iter()
                    .map(|(uuid, map)| {
                        json!({
                            "pool_uuid": uuid.to_simple_ref().to_string(),
                            "devices": <&DeviceSet as Into<Value>>::into(&map),
                        })
                    })
                    .collect(),
            ),
            "hopeless_devices": Value::Array(
                self.hopeless_device_sets
                    .iter()
                    .map(|(uuid, set)| {
                        json!({
                            "pool_uuid": uuid.to_simple_ref().to_string(),
                            "devices": <&DeviceBag as Into<Value>>::into(&set),
                        })
                    })
                    .collect()
            )
        })
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Management of devices which are known to stratisd but not in a pool.

use std::{collections::HashMap, path::PathBuf};

use serde_json::Value;

use devicemapper::Device;

use crate::engine::{
    strat_engine::{
        backstore::{get_metadata, identify_block_device},
        devlinks::setup_pool_devlinks,
        pool::StratPool,
    },
    structures::Table,
    types::{DevUuid, Name, PoolUuid},
};

/// Devices which stratisd has discovered but which have not been assembled
/// into pools.
#[derive(Debug)]
pub struct LiminalDevices {
    errored_pool_devices: HashMap<PoolUuid, HashMap<Device, (DevUuid, PathBuf)>>,
}

impl LiminalDevices {
    pub fn new() -> LiminalDevices {
        LiminalDevices {
            errored_pool_devices: HashMap::new(),
        }
    }

    /// Given a set of devices, try to set up a pool. If the setup fails,
    /// insert the devices into errored_pool_devices. Otherwise, return the pool.
    /// If there is a name conflict between the set of devices in devices
    /// and some existing pool, return an error.
    ///
    /// Precondition: pools.get_by_uuid(pool_uuid).is_none() &&
    ///               self.errored_pool_devices.get(pool_uuid).is_none()
    pub fn try_setup_pool(
        &mut self,
        pools: &Table<StratPool>,
        pool_uuid: PoolUuid,
        devices: HashMap<Device, (DevUuid, PathBuf)>,
    ) -> Option<(Name, StratPool)> {
        assert!(pools.get_by_uuid(pool_uuid).is_none());
        assert!(self.errored_pool_devices.get(&pool_uuid).is_none());

        // Setup a pool from constituent devices in the context of some already
        // setup pools.
        // Return None if the pool's metadata was not found. This is a
        // legitimate non-error condition, which may result if only a subset
        // of the pool's devices are in the set of devices being used.
        // Return an error on all other errors. Note that any one of these
        // errors could represent a temporary condition, that could be changed
        // by finding another device. So it is reasonable to treat them all
        // as loggable at the warning level, but not at the error level.
        // Precondition: every device in devices has already been determined to belong
        // to the pool with pool_uuid.
        fn setup_pool(
            pools: &Table<StratPool>,
            pool_uuid: PoolUuid,
            devices: &HashMap<Device, (DevUuid, PathBuf)>,
        ) -> Result<Option<(Name, StratPool)>, String> {
            let (timestamp, metadata) = match get_metadata(pool_uuid, devices) {
                Err(err) => return Err(format!(
                        "There was an error encountered when reading the metadata for the devices found for pool with UUID {}: {}",
                        pool_uuid.to_simple_ref(),
                        err)),
                Ok(None) => return Ok(None),
                Ok(Some((timestamp, metadata))) => (timestamp, metadata),
            };

            if let Some((uuid, _)) = pools.get_by_name(&metadata.name) {
                return Err(format!(
                        "There is a pool name conflict. The devices currently being processed have been identified as belonging to the pool with UUID {} and name {}, but a pool with the same name and UUID {} is already active",
                        pool_uuid.to_simple_ref(),
                        &metadata.name,
                        uuid.to_simple_ref()));
            }

            StratPool::setup(pool_uuid, devices, timestamp, &metadata)
                .map_err(|err| {
                    format!(
                        "An attempt to set up pool with UUID {} from the assembled devices failed: {}",
                        pool_uuid.to_simple_ref(),
                        err
                    )
                })
                .map(Some)
        }

        let result = setup_pool(pools, pool_uuid, &devices);

        if let Err(err) = &result {
            warn!("{}", err);
        }

        match result {
            Ok(Some((pool_name, pool))) => {
                setup_pool_devlinks(&pool_name, &pool);
                info!(
                    "Pool with name \"{}\" and UUID \"{}\" set up",
                    pool_name,
                    pool_uuid.to_simple_ref()
                );
                Some((pool_name, pool))
            }
            _ => {
                self.errored_pool_devices.insert(pool_uuid, devices);
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
        identify_block_device(event.device()).and_then(move |info| {
            let pool_uuid = info.identifiers.pool_uuid;
            if pools.contains_uuid(pool_uuid) {
                // FIXME: There is the possibilty of an error condition here,
                // if the device found is not in the already set up pool.
                None
            } else {
                let mut devices = self
                    .errored_pool_devices
                    .remove(&pool_uuid)
                    .unwrap_or_else(HashMap::new);

                if devices
                    .insert(
                        info.device_number,
                        (info.identifiers.device_uuid, info.devnode),
                    )
                    .is_none()
                {
                    info!(
                        "Stratis block device with device number \"{}\", pool UUID \"{}\", and device UUID \"{}\" discovered, i.e., identified for the first time during this execution of stratisd",
                        info.device_number,
                        info.identifiers.pool_uuid.to_simple_ref(),
                        info.identifiers.device_uuid.to_simple_ref(),
                    );
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
                self.try_setup_pool(pools, pool_uuid, devices).map(|(name, pool)| (pool_uuid, name, pool))
            }
        })
    }

    /// Generate a JSON report giving some information about the internals
    /// of these devices.
    pub fn report(&self) -> Value {
        Value::Array(self.errored_pool_devices.iter().map(|(uuid, map)| {
            json!({
                "pool_uuid": uuid.to_simple_ref().to_string(),
                "devices": Value::Array(map.values().map(|(_, p)| Value::from(p.display().to_string())).collect()),
            })
        }).collect())
    }
}

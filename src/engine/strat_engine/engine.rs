// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::clone::Clone;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use devicemapper::{Device, DmNameBuf};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::devlinks;
use super::super::engine::{Engine, Eventable, Pool};
use super::super::event::{get_engine_listener_list, EngineEvent};
use super::super::structures::Table;
use super::super::types::{Name, PoolUuid, Redundancy, RenameAction};

use super::backstore::device::is_stratis_device;
use super::backstore::{find_all, get_metadata};
#[cfg(test)]
use super::cleanup::teardown_pools;
use super::cmd::verify_binaries;
use super::dm::{get_dm, get_dm_init};
use super::pool::{check_metadata, StratPool};

const REQUIRED_DM_MINOR_VERSION: u32 = 37;

/// Setup a pool from constituent devices in the context of some already
/// setup pools. Return an error on anything that prevents the pool
/// being set up.
/// Precondition: every device in devices has already been determined to belong
/// to the pool with pool_uuid.
pub fn setup_pool(
    pool_uuid: PoolUuid,
    devices: &HashMap<Device, PathBuf>,
    pools: &Table<StratPool>,
) -> StratisResult<(Name, StratPool)> {
    // FIXME: In this method, various errors are assembled from various
    // sources and combined into strings, so that they
    // can be printed as log messages if necessary. Instead, some kind of
    // error-chaining should be used here and if it is necessary
    // to log the error, the log code should be able to reduce the error
    // chain to something that can be sensibly logged.
    let info_string = || {
        let dev_paths = devices
            .values()
            .map(|p| p.to_str().expect("Unix is utf-8"))
            .collect::<Vec<&str>>()
            .join(" ,");
        format!("(pool UUID: {}, devnodes: {})", pool_uuid, dev_paths)
    };

    let metadata = get_metadata(pool_uuid, devices)?.ok_or_else(|| {
        let err_msg = format!("no metadata found for {}", info_string());
        StratisError::Engine(ErrorEnum::NotFound, err_msg)
    })?;

    if pools.contains_name(&metadata.name) {
        let err_msg = format!(
            "pool with name \"{}\" set up; metadata specifies same name for {}",
            &metadata.name,
            info_string()
        );
        return Err(StratisError::Engine(ErrorEnum::AlreadyExists, err_msg));
    }

    check_metadata(&metadata)
        .or_else(|e| {
            let err_msg = format!(
                "inconsistent metadata for {}: reason: {:?}",
                info_string(),
                e
            );
            Err(StratisError::Engine(ErrorEnum::Error, err_msg))
        })
        .and_then(|_| {
            StratPool::setup(pool_uuid, devices, &metadata).or_else(|e| {
                let err_msg = format!(
                    "failed to set up pool for {}: reason: {:?}",
                    info_string(),
                    e
                );
                Err(StratisError::Engine(ErrorEnum::Error, err_msg))
            })
        })
}

#[derive(Debug)]
pub struct StratEngine {
    pools: Table<StratPool>,

    // Map of stratis devices that have been found but one or more stratis block devices are missing
    // which prevents the associated pools from being setup.
    incomplete_pools: HashMap<PoolUuid, HashMap<Device, PathBuf>>,

    // Maps name of DM devices we are watching to the most recent event number
    // we've handled for each
    watched_dev_last_event_nrs: HashMap<DmNameBuf, u32>,
}

impl StratEngine {
    /// Setup a StratEngine.
    /// 1. Verify the existence of Stratis /dev directory.
    /// 2. Setup all the pools belonging to the engine.
    ///    a. Places any devices which belong to a pool, but are not complete
    ///       in the incomplete pools data structure.
    ///
    /// Returns an error if the kernel doesn't support required DM features.
    /// Returns an error if there was an error reading device nodes.
    /// Returns an error if the binaries on which it depends can not be found.
    pub fn initialize() -> StratisResult<StratEngine> {
        let dm = get_dm_init()?;
        verify_binaries()?;
        let minor_dm_version = dm.version()?.1;
        if minor_dm_version < REQUIRED_DM_MINOR_VERSION {
            let err_msg = format!(
                "Requires DM minor version {} but kernel only supports {}",
                REQUIRED_DM_MINOR_VERSION, minor_dm_version
            );
            return Err(StratisError::Engine(ErrorEnum::Error, err_msg));
        }

        devlinks::setup_dev_path()?;

        let pools = find_all()?;

        let mut table = Table::default();
        let mut incomplete_pools = HashMap::new();
        for (pool_uuid, devices) in pools {
            match setup_pool(pool_uuid, &devices, &table) {
                Ok((pool_name, pool)) => {
                    table.insert(pool_name, pool_uuid, pool);
                }
                Err(err) => {
                    warn!("no pool set up, reason: {:?}", err);
                    incomplete_pools.insert(pool_uuid, devices);
                }
            }
        }

        let engine = StratEngine {
            pools: table,
            incomplete_pools,
            watched_dev_last_event_nrs: HashMap::new(),
        };

        devlinks::setup_devlinks(engine.pools().iter());

        Ok(engine)
    }

    /// Teardown Stratis, preparatory to a shutdown.
    #[cfg(test)]
    pub fn teardown(self) -> StratisResult<()> {
        teardown_pools(self.pools)
    }
}

impl Engine for StratEngine {
    fn create_pool(
        &mut self,
        name: &str,
        blockdev_paths: &[&Path],
        redundancy: Option<u16>,
        force: bool,
    ) -> StratisResult<PoolUuid> {
        let redundancy = calculate_redundancy!(redundancy);

        if self.pools.contains_name(name) {
            return Err(StratisError::Engine(ErrorEnum::AlreadyExists, name.into()));
        }

        let (uuid, pool) = StratPool::initialize(name, blockdev_paths, redundancy, force)?;

        let name = Name::new(name.to_owned());
        devlinks::pool_added(&name);
        self.pools.insert(name, uuid, pool);
        Ok(uuid)
    }

    /// Evaluate a device node & devicemapper::Device to see if it's a valid
    /// stratis device.  If all the devices are present in the pool and the pool isn't already
    /// up and running, it will get setup and the pool uuid will be returned.
    ///
    /// Returns an error if the status of the block device can not be evaluated.
    /// Logs a warning if the block devices appears to be a Stratis block
    /// device and no pool is set up.
    fn block_evaluate(
        &mut self,
        device: Device,
        dev_node: PathBuf,
    ) -> StratisResult<Option<PoolUuid>> {
        let pool_uuid = if let Some((pool_uuid, device_uuid)) = is_stratis_device(&dev_node)? {
            if self.pools.contains_uuid(pool_uuid) {
                // We can get udev events for devices that are already in the pool.  Lets check
                // to see if this block device is already in this existing pool.  If it is, then all
                // is well.  If it's not then we have what is documented below.
                //
                // TODO: Handle the case where we have found a device for an already active pool
                // ref. https://github.com/stratis-storage/stratisd/issues/748

                let (name, pool) = self.pools
                    .get_by_uuid(pool_uuid)
                    .expect("pools.contains_uuid(pool_uuid)");

                match pool.get_strat_blockdev(device_uuid) {
                    None => {
                        error!(
                            "we have a block device {:?} with pool {}, uuid = {} device uuid = {} \
                             which believes it belongs in this pool, but existing active pool has \
                             no knowledge of it",
                            dev_node, name, pool_uuid, device_uuid
                        );
                    }
                    Some((_tier, block_dev)) => {
                        // Make sure that this block device and existing block device refer to the
                        // same physical device that's already in the pool
                        if device != *block_dev.device() {
                            error!(
                                "we have a block device with the same uuid as one already in the \
                                 pool, but the one in the pool has device number {:}, \
                                 while the one just found has device number {:}",
                                block_dev.device(),
                                device,
                            );
                        }
                    }
                }
                None
            } else {
                let mut devices = self.incomplete_pools
                    .remove(&pool_uuid)
                    .or_else(|| Some(HashMap::new()))
                    .expect("We just retrieved or created a HashMap");
                devices.insert(device, dev_node);
                match setup_pool(pool_uuid, &devices, &self.pools) {
                    Ok((pool_name, pool)) => {
                        devlinks::setup_pool_devlinks(&pool_name, &pool);
                        self.pools.insert(pool_name, pool_uuid, pool);
                        Some(pool_uuid)
                    }
                    Err(err) => {
                        warn!("no pool set up, reason: {:?}", err);
                        self.incomplete_pools.insert(pool_uuid, devices);
                        None
                    }
                }
            }
        } else {
            None
        };
        Ok(pool_uuid)
    }

    fn destroy_pool(&mut self, uuid: PoolUuid) -> StratisResult<bool> {
        if let Some((_, pool)) = self.pools.get_by_uuid(uuid) {
            if pool.has_filesystems() {
                return Err(StratisError::Engine(
                    ErrorEnum::Busy,
                    "filesystems remaining on pool".into(),
                ));
            };
        } else {
            return Ok(false);
        }

        let (pool_name, mut pool) = self.pools
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pools.get_by_uuid() returned a value");

        pool.destroy()?;
        devlinks::pool_removed(&pool_name);
        Ok(true)
    }

    fn rename_pool(&mut self, uuid: PoolUuid, new_name: &str) -> StratisResult<RenameAction> {
        let old_name = rename_pool_pre!(self; uuid; new_name);

        let (_, mut pool) = self.pools
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pools.get_by_uuid() returned a value");

        let new_name = Name::new(new_name.to_owned());
        if let Err(err) = pool.write_metadata(&new_name) {
            self.pools.insert(old_name, uuid, pool);
            Err(err)
        } else {
            get_engine_listener_list().notify(&EngineEvent::PoolRenamed {
                dbus_path: pool.get_dbus_path(),
                from: &*old_name,
                to: &*new_name,
            });

            self.pools.insert(new_name.clone(), uuid, pool);
            devlinks::pool_renamed(&old_name, &new_name);
            Ok(RenameAction::Renamed)
        }
    }

    fn get_pool(&self, uuid: PoolUuid) -> Option<(Name, &Pool)> {
        get_pool!(self; uuid)
    }

    fn get_mut_pool(&mut self, uuid: PoolUuid) -> Option<(Name, &mut Pool)> {
        get_mut_pool!(self; uuid)
    }

    fn configure_simulator(&mut self, _denominator: u32) -> StratisResult<()> {
        Ok(()) // we're not the simulator and not configurable, so just say ok
    }

    fn pools(&self) -> Vec<(Name, PoolUuid, &Pool)> {
        self.pools
            .iter()
            .map(|(name, uuid, pool)| (name.clone(), *uuid, pool as &Pool))
            .collect()
    }

    fn pools_mut(&mut self) -> Vec<(Name, PoolUuid, &mut Pool)> {
        self.pools
            .iter_mut()
            .map(|(name, uuid, pool)| (name.clone(), *uuid, pool as &mut Pool))
            .collect()
    }

    fn get_eventable(&self) -> Option<&'static Eventable> {
        Some(get_dm())
    }

    fn evented(&mut self) -> StratisResult<()> {
        let device_list: HashMap<_, _> = get_dm()
            .list_devices()?
            .into_iter()
            .map(|(dm_name, _, event_nr)| {
                (
                    dm_name,
                    event_nr.expect("Supported DM versions always provide a value"),
                )
            })
            .collect();

        for (pool_name, pool_uuid, pool) in &mut self.pools {
            for dm_name in pool.get_eventing_dev_names(*pool_uuid) {
                if device_list.get(&dm_name) > self.watched_dev_last_event_nrs.get(&dm_name) {
                    pool.event_on(*pool_uuid, pool_name, &dm_name)?;
                }
            }
        }

        self.watched_dev_last_event_nrs = device_list;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::fs::remove_dir_all;

    use engine::engine::DEV_PATH;

    use super::super::tests::{loopbacked, real};

    use super::*;

    /// Verify that a pool rename causes the pool metadata to get the new name.
    fn test_pool_rename(paths: &[&Path]) {
        let mut engine = StratEngine::initialize().unwrap();

        let name1 = "name1";
        let uuid1 = engine.create_pool(&name1, paths, None, false).unwrap();

        let name2 = "name2";
        let action = engine.rename_pool(uuid1, name2).unwrap();

        assert_eq!(action, RenameAction::Renamed);
        engine.teardown().unwrap();

        let engine = StratEngine::initialize().unwrap();
        let pool_name: String = engine.get_pool(uuid1).unwrap().0.to_owned();
        assert_eq!(pool_name, name2);
    }

    #[test]
    pub fn loop_test_pool_rename() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(1, 3, None),
            test_pool_rename,
        );
    }

    #[test]
    pub fn real_test_pool_rename() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1, None, None), test_pool_rename);
    }

    /// Test engine setup.
    /// 1. Create two pools.
    /// 2. Verify that both exist.
    /// 3. Teardown the engine.
    /// 4. Initialize the engine.
    /// 5. Verify that pools can be found again.
    /// 6. Teardown the engine and remove "/dev/stratis".
    /// 7. Initialize the engine one more time.
    /// 8. Verify that both pools are found and that there are no incomplete pools.
    fn test_setup(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let mut engine = StratEngine::initialize().unwrap();

        let name1 = "name1";
        let uuid1 = engine.create_pool(&name1, paths1, None, false).unwrap();

        let name2 = "name2";
        let uuid2 = engine.create_pool(&name2, paths2, None, false).unwrap();

        assert!(engine.get_pool(uuid1).is_some());
        assert!(engine.get_pool(uuid2).is_some());

        engine.teardown().unwrap();

        let engine = StratEngine::initialize().unwrap();

        assert!(engine.get_pool(uuid1).is_some());
        assert!(engine.get_pool(uuid2).is_some());

        engine.teardown().unwrap();
        remove_dir_all(DEV_PATH).unwrap();

        let engine = StratEngine::initialize().unwrap();
        assert_eq!(engine.incomplete_pools, HashMap::new());

        assert!(engine.get_pool(uuid1).is_some());
        assert!(engine.get_pool(uuid2).is_some());

        engine.teardown().unwrap();
    }

    #[test]
    pub fn loop_test_setup() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(2, 3, None), test_setup);
    }

    #[test]
    pub fn real_test_setup() {
        real::test_with_spec(real::DeviceLimits::AtLeast(2, None, None), test_setup);
    }
}

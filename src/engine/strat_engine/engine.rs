// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate libc;

use std::collections::HashMap;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};

use devicemapper::{DM, Device, DmNameBuf};

use super::super::engine::{Engine, Eventable, HasName, HasUuid, Pool};
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::structures::Table;
use super::super::types::{DevUuid, PoolUuid, Redundancy, RenameAction};

use super::cleanup::teardown_pools;
use super::devlinks;
use super::pool::StratPool;
use super::setup::{find_all, is_stratis_device};

const REQUIRED_DM_MINOR_VERSION: u32 = 37;

#[derive(Debug, PartialEq, Eq)]
pub enum DevOwnership {
    Ours(PoolUuid, DevUuid),
    Unowned,
    Theirs,
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

impl Eventable for DM {
    /// Get file we'd like to have monitored for activity
    fn get_pollable_fd(&mut self) -> RawFd {
        self.file().as_raw_fd()
    }

    fn clear_event(&mut self) -> EngineResult<()> {
        self.arm_poll()?;

        Ok(())
    }
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
    /// Returns an error and tears down all the pools if we find a pool with a
    /// duplicate name found.
    pub fn initialize() -> EngineResult<StratEngine> {
        let dm = DM::new()?;
        let minor_dm_version = dm.version()?.1;
        if minor_dm_version < REQUIRED_DM_MINOR_VERSION {
            let err_msg = format!("Requires DM minor version {} but kernel only supports {}",
                                  REQUIRED_DM_MINOR_VERSION,
                                  minor_dm_version);
            return Err(EngineError::Engine(ErrorEnum::Error, err_msg));
        }

        let pools = find_all()?;

        let mut table = Table::default();
        let mut incomplete_pools = HashMap::new();

        for (pool_uuid, devices) in &pools {
            match StratPool::setup(&dm, *pool_uuid, devices) {
                Ok(pool) => {
                    let evicted = table.insert(pool);
                    if !evicted.is_empty() {
                        // TODO: update state machine on failure.
                        let _ = teardown_pools(table.empty());
                        let err_msg = "found two pools with the same name";
                        return Err(EngineError::Engine(ErrorEnum::Invalid, err_msg.into()));
                    }
                }
                Err(_) => {
                    // Unable to setup pool, it might become complete later.
                    incomplete_pools.insert(*pool_uuid, devices.clone());
                }
            }
        }

        let engine = StratEngine {
            pools: table,
            incomplete_pools,
            watched_dev_last_event_nrs: HashMap::new(),
        };

        devlinks::setup_devlinks(engine.pools().into_iter())?;

        Ok(engine)
    }

    /// Teardown Stratis, preparatory to a shutdown.
    pub fn teardown(self) -> EngineResult<()> {
        teardown_pools(self.pools.empty())
    }
}

impl Engine for StratEngine {
    fn create_pool(&mut self,
                   name: &str,
                   blockdev_paths: &[&Path],
                   redundancy: Option<u16>,
                   force: bool)
                   -> EngineResult<PoolUuid> {

        let redundancy = calculate_redundancy!(redundancy);

        if self.pools.contains_name(name) {
            return Err(EngineError::Engine(ErrorEnum::AlreadyExists, name.into()));
        }

        let dm = DM::new()?;
        let pool = StratPool::initialize(&dm, name, blockdev_paths, redundancy, force)?;

        let uuid = pool.uuid();
        devlinks::pool_added(&*pool.name())?;
        self.pools.insert(pool);

        Ok(uuid)
    }

    /// Evaluate a device node & devicemapper::Device to see if it's a valid
    /// stratis device.  If all the devices are present in the pool and the pool isn't already
    /// up and running, it will get setup and the pool uuid will be returned.
    ///
    /// Returns an error if the status of the block device can not be evaluated.
    fn block_evaluate(&mut self,
                      dev_node: PathBuf,
                      device: Device)
                      -> EngineResult<Option<PoolUuid>> {
        let dm = DM::new()?;
        if let Some(pool_uuid) = is_stratis_device(&dev_node)? {
            if self.pools.contains_uuid(pool_uuid) {
                // TODO: Handle the case where we have found a device for an already active pool
                // ref. https://github.com/stratis-storage/stratisd/issues/748
                warn!("udev add: pool {} is already known, ignoring device {:?}!",
                      pool_uuid,
                      dev_node);
                Ok(None)
            } else {

                let mut devices = self.incomplete_pools
                    .remove(&pool_uuid)
                    .or_else(|| Some(HashMap::new()))
                    .expect("We just retrieved or created a HashMap");
                devices.insert(device, dev_node);

                let rc = if let Ok(pool) = StratPool::setup(&dm, pool_uuid, &devices) {
                    // We know we have a unique uuid, but the pools table requires a unique name too
                    // so we will ensure unique before we insert as we don't want to change the
                    // existing state if we have a conflict.
                    //
                    // We will also _not_ exit the daemon as we do in initialize as we were
                    // previously up and running for some duration of time.
                    if !self.pools.contains_name(&*pool.name()) {
                        self.pools.insert(pool);
                        Some(pool_uuid)
                    } else {
                        let dev_paths = devices
                            .values()
                            .map(|p| p.to_str().expect("Expecting valid device path!"))
                            .collect::<Vec<&str>>()
                            .join(", ");

                        self.incomplete_pools.insert(pool_uuid, devices);

                        error!("udev add: duplicate pool name {:?} for uuid {:?}, \
                                devices[{}], failing to setup complete pool!",
                               pool.name(),
                               pool_uuid,
                               dev_paths);
                        if let Err(e) = pool.teardown() {
                            error!("Error while tearing down pool with duplicate name! {:?}!",
                                   e);
                        }
                        None
                    }
                } else {
                    self.incomplete_pools.insert(pool_uuid, devices);
                    None
                };
                Ok(rc)
            }
        } else {
            Ok(None)
        }
    }

    fn destroy_pool(&mut self, uuid: PoolUuid) -> EngineResult<bool> {
        if let Some(pool) = self.pools.get_by_uuid(uuid) {
            if pool.has_filesystems() {
                return Err(EngineError::Engine(ErrorEnum::Busy,
                                               "filesystems remaining on pool".into()));
            };
        } else {
            return Ok(false);
        }

        let pool = self.pools
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pools.get_by_uuid() returned a value");
        let pool_name = pool.name().to_owned();
        pool.destroy()?;
        devlinks::pool_removed(&pool_name)?;
        Ok(true)
    }

    fn rename_pool(&mut self, uuid: PoolUuid, new_name: &str) -> EngineResult<RenameAction> {
        let old_name = rename_pool_pre!(self; uuid; new_name);

        let mut pool = self.pools
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pools.get_by_uuid() returned a value");
        pool.rename(new_name);

        if let Err(err) = pool.write_metadata() {
            pool.rename(&old_name);
            self.pools.insert(pool);
            Err(err)
        } else {
            self.pools.insert(pool);
            devlinks::pool_renamed(&old_name, new_name)?;
            Ok(RenameAction::Renamed)
        }
    }

    fn get_pool(&self, uuid: PoolUuid) -> Option<&Pool> {
        get_pool!(self; uuid)
    }

    fn get_mut_pool(&mut self, uuid: PoolUuid) -> Option<&mut Pool> {
        get_mut_pool!(self; uuid)
    }

    fn configure_simulator(&mut self, _denominator: u32) -> EngineResult<()> {
        Ok(()) // we're not the simulator and not configurable, so just say ok
    }

    fn check(&mut self) -> () {
        check_engine!(self);
    }

    fn pools(&self) -> Vec<&Pool> {
        self.pools.into_iter().map(|x| x as &Pool).collect()
    }

    fn get_eventable(&mut self) -> EngineResult<Option<Box<Eventable>>> {
        Ok(Some(Box::new(DM::new()?)))
    }

    fn evented(&mut self) -> EngineResult<()> {
        let device_list: HashMap<_, _> = DM::new()?
            .list_devices()?
            .into_iter()
            .map(|(dm_name, _, event_nr)| {
                     (dm_name, event_nr.expect("Supported DM versions always provide a value"))
                 })
            .collect();

        for pool in &mut self.pools {
            for dm_name in pool.get_eventing_dev_names() {
                if device_list.get(&dm_name) > self.watched_dev_last_event_nrs.get(&dm_name) {
                    pool.event_on(&dm_name)?;
                }
            }
        }

        self.watched_dev_last_event_nrs = device_list;

        Ok(())
    }
}

#[cfg(test)]
mod test {
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
        let pool_name: String = engine.get_pool(uuid1).unwrap().name().to_owned();
        assert_eq!(pool_name, name2);
    }

    #[test]
    pub fn loop_test_pool_rename() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(1, 3), test_pool_rename);
    }

    #[test]
    pub fn real_test_pool_rename() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1), test_pool_rename);
    }

    /// Test engine setup.
    /// 1. Create two pools.
    /// 2. Verify that both exist.
    /// 3. Teardown the engine.
    /// 4. Verify that pools are gone.
    /// 5. Initialize the engine.
    /// 6. Verify that pools can be found again.
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
    }

    #[test]
    pub fn loop_test_setup() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(2, 3), test_setup);
    }

    #[test]
    pub fn real_test_setup() {
        real::test_with_spec(real::DeviceLimits::AtLeast(2), test_setup);
    }
}

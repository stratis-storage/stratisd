// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{clone::Clone, collections::HashMap, path::Path};

use serde_json::Value;

use devicemapper::DmNameBuf;

use crate::{
    engine::{
        engine::{Eventable, KeyActions},
        event::get_engine_listener_list,
        shared::{create_pool_idempotent_or_err, validate_name, validate_paths},
        strat_engine::{
            backstore::CryptHandle,
            cmd::verify_binaries,
            devlinks,
            dm::{get_dm, get_dm_init},
            keys::{MemoryFilesystem, MemoryPrivateFilesystem, StratKeyActions},
            liminal::{find_all, LiminalDevices},
            names::KeyDescription,
            pool::StratPool,
        },
        structures::Table,
        types::{
            BlockDevTier, CreateAction, DeleteAction, DevUuid, RenameAction, ReportType,
            SetUnlockAction, UnlockMethod,
        },
        Engine, EngineEvent, Name, Pool, PoolUuid, Report,
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

const REQUIRED_DM_MINOR_VERSION: u32 = 37;

#[derive(Debug)]
pub struct StratEngine {
    pools: Table<StratPool>,

    // Maps pool UUIDs to information about sets of devices that are
    // associated with that UUID but have not been converted into a pool.
    liminal_devices: LiminalDevices,

    // Maps name of DM devices we are watching to the most recent event number
    // we've handled for each
    watched_dev_last_event_nrs: HashMap<DmNameBuf, u32>,

    // Handler for key operations
    key_handler: StratKeyActions,

    // TODO: Remove this code when Clevis supports reading keys from the
    // kernel keyring.
    // In memory filesystem for passing keys to Clevis.
    key_fs: MemoryFilesystem,
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

        let mut liminal_devices = LiminalDevices::default();
        let mut pools = Table::default();
        for (pool_name, pool_uuid, pool) in liminal_devices.setup_pools(find_all()?) {
            pools.insert(pool_name, pool_uuid, pool);
        }

        Ok(StratEngine {
            pools,
            liminal_devices,
            watched_dev_last_event_nrs: HashMap::new(),
            key_handler: StratKeyActions,
            key_fs: MemoryFilesystem::new()?,
        })
    }

    /// Recursively remove all devicemapper devices in all pools.
    /// Do not remove the dm-crypt devices that comprise the backstore.
    #[cfg(test)]
    pub fn teardown(self) -> StratisResult<()> {
        let mut untorndown_pools = Vec::new();
        for (_, uuid, mut pool) in self.pools {
            pool.teardown()
                .unwrap_or_else(|_| untorndown_pools.push(uuid));
        }
        if untorndown_pools.is_empty() {
            Ok(())
        } else {
            let err_msg = format!(
                "Failed to teardown already set up pools: {:?}",
                untorndown_pools
            );
            Err(StratisError::Engine(ErrorEnum::Error, err_msg))
        }
    }
}

impl<'a> Into<Value> for &'a StratEngine {
    // Precondition: (&StratPool).into() pattern matches Value::Object(_)
    // Precondition: (&LiminalDevices).into() pattern matches Value::Object(_)
    fn into(self) -> Value {
        let json = json!({
            "pools": Value::Array(
                self.pools.iter()
                    .map(|(name, uuid, pool)| {
                        let mut json = json!({
                            "uuid": Value::from(uuid.to_simple_ref().to_string()),
                            "name": Value::from(name.to_string()),
                        });
                        if let Value::Object(ref mut map) = json {
                            map.extend(
                                if let Value::Object(map) = <&StratPool as Into<Value>>::into(pool) {
                                    map.into_iter()
                                } else {
                                    unreachable!("StratPool conversion returns a JSON object");
                                }
                            );
                        } else {
                            unreachable!("json!() always creates a JSON object")
                        }
                        json
                    })
                    .collect()
            ),
        });
        if let (Value::Object(mut j), Value::Object(map)) = (
            json,
            <&LiminalDevices as Into<Value>>::into(&self.liminal_devices),
        ) {
            j.extend(map.into_iter());
            Value::Object(j)
        } else {
            unreachable!("json!() and LiminalDevices::into() always return JSON object");
        }
    }
}

impl Report for StratEngine {
    fn get_report(&self, report_type: ReportType) -> Value {
        match report_type {
            ReportType::ErroredPoolDevices => (&self.liminal_devices).into(),
            ReportType::EngineState => self.into(),
        }
    }
}

impl Engine for StratEngine {
    fn handle_event(&mut self, event: &libudev::Event) -> Option<(Name, PoolUuid, &mut dyn Pool)> {
        if let Some((pool_uuid, pool_name, pool)) =
            self.liminal_devices.block_evaluate(&self.pools, event)
        {
            self.pools.insert(pool_name.clone(), pool_uuid, pool);
            Some((
                pool_name,
                pool_uuid,
                self.pools
                    .get_mut_by_uuid(pool_uuid)
                    .expect("just_inserted")
                    .1 as &mut dyn Pool,
            ))
        } else {
            None
        }
    }

    fn create_pool(
        &mut self,
        name: &str,
        blockdev_paths: &[&Path],
        redundancy: Option<u16>,
        key_desc: Option<KeyDescription>,
    ) -> StratisResult<CreateAction<PoolUuid>> {
        let redundancy = calculate_redundancy!(redundancy);

        validate_name(name)?;

        validate_paths(blockdev_paths)?;

        match self.pools.get_by_name(name) {
            Some((_, pool)) => create_pool_idempotent_or_err(pool, name, blockdev_paths),
            None => {
                if blockdev_paths.is_empty() {
                    Err(StratisError::Engine(
                        ErrorEnum::Invalid,
                        "At least one blockdev is required to create a pool.".to_string(),
                    ))
                } else {
                    let (uuid, pool) =
                        StratPool::initialize(name, blockdev_paths, redundancy, key_desc.as_ref())?;

                    let name = Name::new(name.to_owned());
                    self.pools.insert(name, uuid, pool);
                    Ok(CreateAction::Created(uuid))
                }
            }
        }
    }

    fn destroy_pool(&mut self, uuid: PoolUuid) -> StratisResult<DeleteAction<PoolUuid>> {
        if let Some((_, pool)) = self.pools.get_by_uuid(uuid) {
            if pool.has_filesystems() {
                return Err(StratisError::Engine(
                    ErrorEnum::Busy,
                    "filesystems remaining on pool".into(),
                ));
            };
        } else {
            return Ok(DeleteAction::Identity);
        }

        let (pool_name, mut pool) = self
            .pools
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pools.get_by_uuid() returned a value");

        if let Err(err) = pool.destroy() {
            self.pools.insert(pool_name, uuid, pool);
            Err(err)
        } else {
            Ok(DeleteAction::Deleted(uuid))
        }
    }

    fn rename_pool(
        &mut self,
        uuid: PoolUuid,
        new_name: &str,
    ) -> StratisResult<RenameAction<PoolUuid>> {
        validate_name(new_name)?;
        let old_name = rename_pool_pre_idem!(self; uuid; new_name);

        let (_, mut pool) = self
            .pools
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
            if let Err(e) = devlinks::pool_renamed(&old_name) {
                warn!("Pool rename symlink action failed: {}", e)
            };
            Ok(RenameAction::Renamed(uuid))
        }
    }

    fn unlock_pool(
        &mut self,
        pool_uuid: PoolUuid,
        unlock_method: UnlockMethod,
    ) -> StratisResult<SetUnlockAction<DevUuid>> {
        let unlocked = self
            .liminal_devices
            .unlock_pool(&self.pools, pool_uuid, unlock_method)?;
        Ok(SetUnlockAction::new(unlocked))
    }

    fn clevis_bind_pool(
        &self,
        pool_uuid: PoolUuid,
        key_desc: &KeyDescription,
        tang_url: &str,
    ) -> StratisResult<()> {
        if let Some((_, pool)) = self.get_pool(pool_uuid) {
            let mut rollback_record = Vec::new();
            let key_fs = MemoryPrivateFilesystem::new()?;
            for (_uuid, tier, dev) in pool.blockdevs() {
                if tier == BlockDevTier::Data {
                    let result = key_fs.key_op(key_desc, |keyfile_path| {
                        let path = dev.devnode().physical_path();
                        if let Some(mut handle) = CryptHandle::setup(path)? {
                            let res = handle
                                .clevis_bind(keyfile_path, tang_url)
                                .map_err(StratisError::Crypt);
                            if res.is_ok() {
                                rollback_record.push(path);
                            }
                            res
                        } else {
                            Err(StratisError::Error(format!(
                                "Failed to acquire cryptsetup context for device {} \
                                when attempting the clevis binding operation.",
                                path.display(),
                            )))
                        }
                    });
                    if result.is_err() {
                        rollback_record.into_iter().for_each(|path| {
                            if let Ok(Some(mut handle)) = CryptHandle::setup(path) {
                                if let Err(e) = handle.clevis_unbind() {
                                    warn!(
                                        "Failed to unbind device {} from clevis during \
                                        rollback: {}",
                                        path.display(),
                                        e,
                                    );
                                }
                            } else {
                                warn!(
                                    "Failed to acquire cryptsetup context for device {} \
                                    when rolling back clevis binding operation.",
                                    path.display(),
                                );
                            }
                        });
                        return result;
                    }
                }
            }
            Ok(())
        } else {
            Err(StratisError::Engine(
                ErrorEnum::NotFound,
                format!("Pool with UUID {} not found", pool_uuid.to_simple_ref()),
            ))
        }
    }

    fn get_pool(&self, uuid: PoolUuid) -> Option<(Name, &dyn Pool)> {
        get_pool!(self; uuid)
    }

    fn get_mut_pool(&mut self, uuid: PoolUuid) -> Option<(Name, &mut dyn Pool)> {
        get_mut_pool!(self; uuid)
    }

    fn locked_pools(&self) -> HashMap<PoolUuid, KeyDescription> {
        self.liminal_devices.locked_pools()
    }

    fn configure_simulator(&mut self, _denominator: u32) -> StratisResult<()> {
        Ok(()) // we're not the simulator and not configurable, so just say ok
    }

    fn pools(&self) -> Vec<(Name, PoolUuid, &dyn Pool)> {
        self.pools
            .iter()
            .map(|(name, uuid, pool)| (name.clone(), *uuid, pool as &dyn Pool))
            .collect()
    }

    fn pools_mut(&mut self) -> Vec<(Name, PoolUuid, &mut dyn Pool)> {
        self.pools
            .iter_mut()
            .map(|(name, uuid, pool)| (name.clone(), *uuid, pool as &mut dyn Pool))
            .collect()
    }

    fn get_eventable(&self) -> Option<&'static dyn Eventable> {
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

    fn get_key_handler(&self) -> &dyn KeyActions {
        &self.key_handler as &dyn KeyActions
    }

    fn get_key_handler_mut(&mut self) -> &mut dyn KeyActions {
        &mut self.key_handler as &mut dyn KeyActions
    }
}

#[cfg(test)]
mod test {
    use crate::engine::strat_engine::tests::{loopbacked, real};

    use crate::engine::types::EngineAction;

    use super::*;

    /// Verify that a pool rename causes the pool metadata to get the new name.
    fn test_pool_rename(paths: &[&Path]) {
        let mut engine = StratEngine::initialize().unwrap();

        let name1 = "name1";
        let uuid1 = engine
            .create_pool(name1, paths, None, None)
            .unwrap()
            .changed()
            .unwrap();

        let name2 = "name2";
        let action = engine.rename_pool(uuid1, name2).unwrap();

        assert_eq!(action, RenameAction::Renamed(uuid1));
        engine.teardown().unwrap();

        let engine = StratEngine::initialize().unwrap();
        let pool_name: String = engine.get_pool(uuid1).unwrap().0.to_owned();
        assert_eq!(pool_name, name2);
    }

    #[test]
    fn loop_test_pool_rename() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_pool_rename,
        );
    }

    #[test]
    fn real_test_pool_rename() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(1, None, None),
            test_pool_rename,
        );
    }

    /// Test engine setup.
    /// 1. Create two pools.
    /// 2. Verify that both exist.
    /// 3. Teardown the engine.
    /// 4. Initialize the engine.
    /// 5. Verify that pools can be found again.
    /// 6. Teardown the engine
    /// 7. Initialize the engine one more time.
    /// 8. Verify that both pools are found.
    fn test_setup(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let mut engine = StratEngine::initialize().unwrap();

        let name1 = "name1";
        let uuid1 = engine
            .create_pool(name1, paths1, None, None)
            .unwrap()
            .changed()
            .unwrap();

        let name2 = "name2";
        let uuid2 = engine
            .create_pool(name2, paths2, None, None)
            .unwrap()
            .changed()
            .unwrap();

        assert!(engine.get_pool(uuid1).is_some());
        assert!(engine.get_pool(uuid2).is_some());

        engine.teardown().unwrap();

        let engine = StratEngine::initialize().unwrap();

        assert!(engine.get_pool(uuid1).is_some());
        assert!(engine.get_pool(uuid2).is_some());

        engine.teardown().unwrap();

        let engine = StratEngine::initialize().unwrap();

        assert!(engine.get_pool(uuid1).is_some());
        assert!(engine.get_pool(uuid2).is_some());

        engine.teardown().unwrap();
    }

    #[test]
    fn loop_test_setup() {
        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Range(2, 3, None), test_setup);
    }

    #[test]
    fn real_test_setup() {
        real::test_with_spec(&real::DeviceLimits::AtLeast(2, None, None), test_setup);
    }
}

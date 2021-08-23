// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use serde_json::Value;

use devicemapper::DmNameBuf;

use crate::{
    engine::{
        shared::{create_pool_idempotent_or_err, validate_name, validate_paths},
        strat_engine::{
            cmd::verify_binaries,
            dm::get_dm,
            keys::{MemoryFilesystem, StratKeyActions},
            liminal::{find_all, LiminalDevices},
            pool::StratPool,
        },
        structures::Table,
        types::{
            ChangedProperties, CreateAction, DeleteAction, DevUuid, EncryptionInfo, LockedPoolInfo,
            RenameAction, ReportType, SetUnlockAction, UdevEngineEvent, UnlockMethod,
        },
        Engine, Name, PoolUuid, Report,
    },
    stratis::{StratisError, StratisResult},
};

#[derive(Debug)]
pub struct StratEngine {
    pools: Table<PoolUuid, StratPool>,

    // Maps pool UUIDs to information about sets of devices that are
    // associated with that UUID but have not been converted into a pool.
    liminal_devices: LiminalDevices,

    // Maps name of DM devices we are watching to the most recent event number
    // we've handled for each
    watched_dev_last_event_nrs: HashMap<PoolUuid, HashMap<DmNameBuf, u32>>,

    // Handler for key operations
    key_handler: StratKeyActions,

    // TODO: Remove this code when Clevis supports reading keys from the
    // kernel keyring.
    // In memory filesystem for passing keys to Clevis.
    // See GitHub issue: https://github.com/stratis-storage/project/issues/212.
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
        verify_binaries()?;

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
            Err(StratisError::Msg(err_msg))
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
                            "uuid": Value::from(uuid.to_string()),
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
    fn engine_state_report(&self) -> Value {
        self.into()
    }

    fn get_report(&self, report_type: ReportType) -> Value {
        match report_type {
            ReportType::ErroredPoolDevices => (&self.liminal_devices).into(),
        }
    }
}

impl Engine for StratEngine {
    type Pool = StratPool;
    type KeyActions = StratKeyActions;

    fn handle_event(&mut self, event: &UdevEngineEvent) -> Option<(Name, PoolUuid, &Self::Pool)> {
        if let Some((pool_uuid, pool_name, pool)) =
            self.liminal_devices.block_evaluate(&self.pools, event)
        {
            self.pools.insert(pool_name.clone(), pool_uuid, pool);
            Some((
                pool_name,
                pool_uuid,
                self.pools.get_by_uuid(pool_uuid).expect("just_inserted").1,
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
        encryption_info: Option<&EncryptionInfo>,
    ) -> StratisResult<CreateAction<PoolUuid>> {
        let redundancy = calculate_redundancy!(redundancy);

        validate_name(name)?;

        validate_paths(blockdev_paths)?;

        match self.pools.get_by_name(name) {
            Some((_, pool)) => create_pool_idempotent_or_err(pool, name, blockdev_paths),
            None => {
                if blockdev_paths.is_empty() {
                    Err(StratisError::Msg(
                        "At least one blockdev is required to create a pool.".to_string(),
                    ))
                } else {
                    let (uuid, pool) =
                        StratPool::initialize(name, blockdev_paths, redundancy, encryption_info)?;

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
                return Err(StratisError::Msg("filesystems remaining on pool".into()));
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
            self.pools.insert(new_name, uuid, pool);
            let (new_name, pool) = self.pools.get_by_uuid(uuid).expect("Inserted above");
            pool.udev_pool_change(&new_name);
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

    fn get_pool(&self, uuid: PoolUuid) -> Option<(Name, &Self::Pool)> {
        get_pool!(self; uuid)
    }

    fn get_mut_pool(&mut self, uuid: PoolUuid) -> Option<(Name, &mut Self::Pool)> {
        get_mut_pool!(self; uuid)
    }

    fn locked_pools(&self) -> HashMap<PoolUuid, LockedPoolInfo> {
        self.liminal_devices.locked_pools()
    }

    fn pools(&self) -> Vec<(Name, PoolUuid, &Self::Pool)> {
        self.pools
            .iter()
            .map(|(name, uuid, pool)| (name.clone(), *uuid, pool))
            .collect()
    }

    fn pools_mut(&mut self) -> Vec<(Name, PoolUuid, &mut Self::Pool)> {
        self.pools
            .iter_mut()
            .map(|(name, uuid, pool)| (name.clone(), *uuid, pool))
            .collect()
    }

    fn get_events(&mut self) -> StratisResult<Vec<PoolUuid>> {
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

        let mut changed = Vec::new();
        for (_, pool_uuid, pool) in self.pools.iter_mut() {
            let dev_names = pool.get_eventing_dev_names(*pool_uuid);
            let event_nrs = device_list
                .iter()
                .filter_map(|(dm_name, event_nr)| {
                    if dev_names.contains(dm_name) {
                        Some((dm_name.clone(), *event_nr))
                    } else {
                        None
                    }
                })
                .collect::<HashMap<_, _>>();

            if self.watched_dev_last_event_nrs.get(pool_uuid) != Some(&event_nrs) {
                changed.push(*pool_uuid);
            }

            self.watched_dev_last_event_nrs
                .insert(*pool_uuid, event_nrs);
        }

        Ok(changed)
    }

    fn pool_evented(&mut self, pools: Option<&Vec<PoolUuid>>) -> StratisResult<HashSet<PoolUuid>> {
        fn handle_eventing(
            name: &Name,
            uuid: PoolUuid,
            pool: &mut StratPool,
            changed: &mut HashSet<PoolUuid>,
            errors: &mut Vec<StratisError>,
        ) {
            if let Err(e) = pool.event_on(uuid, name) {
                errors.push(e);
            } else {
                changed.insert(uuid);
            }
        }

        let mut changed = HashSet::new();
        let mut errors = Vec::new();

        match pools {
            Some(ps) => {
                for uuid in ps {
                    if let Some((name, pool)) = self.pools.get_mut_by_uuid(*uuid) {
                        handle_eventing(&name, *uuid, pool, &mut changed, &mut errors);
                    } else {
                        errors.push(StratisError::Msg(format!(
                            "Pool with UUID {} could not be found",
                            uuid
                        )));
                    }
                }
            }
            None => {
                for (name, uuid, pool) in self.pools.iter_mut() {
                    handle_eventing(name, *uuid, pool, &mut changed, &mut errors);
                }
            }
        }

        if errors.is_empty() {
            Ok(changed)
        } else {
            let msg = if changed.is_empty() {
                "The following errors were reported while handling devicemapper eventing"
                    .to_string()
            } else {
                format!("Operations on pools with UUIDs {:?} succeeded but the following errors were also reported while handling devicemapper eventing", changed)
            };
            Err(StratisError::BestEffortError(msg, errors))
        }
    }

    fn fs_evented(&mut self, pools: Option<&Vec<PoolUuid>>) -> StratisResult<ChangedProperties> {
        let mut changed = ChangedProperties {
            filesystem_sizes: HashMap::new(),
        };
        let mut errors = Vec::new();

        fn handle_eventing(
            name: &Name,
            uuid: PoolUuid,
            pool: &mut StratPool,
            changed: &mut ChangedProperties,
            errors: &mut Vec<StratisError>,
        ) {
            match pool.fs_event_on(uuid, name) {
                Ok(fs_changed) => {
                    changed.extend(fs_changed);
                }
                Err(e) => {
                    errors.push(e);
                }
            }
        }

        match pools {
            Some(ps) => {
                for uuid in ps {
                    if let Some((name, pool)) = self.pools.get_mut_by_uuid(*uuid) {
                        handle_eventing(&name, *uuid, pool, &mut changed, &mut errors);
                    } else {
                        errors.push(StratisError::Msg(format!(
                            "Pool with UUID {} could not be found",
                            uuid
                        )));
                    }
                }
            }
            None => {
                for (name, uuid, pool) in self.pools.iter_mut() {
                    handle_eventing(name, *uuid, pool, &mut changed, &mut errors);
                }
            }
        }

        if errors.is_empty() {
            Ok(changed)
        } else {
            let msg = if changed.is_changed() {
                format!(
                    "Operations on filesystems with UUIDs {:?} succeeded but the following errors were also reported while handling devicemapper eventing for filesystems",
                    changed.filesystem_sizes.keys().collect::<Vec<_>>(),
                )
            } else {
                "The following errors were reported while handling devicemapper eventing for filesystems".to_string()
            };
            Err(StratisError::BestEffortError(msg, errors))
        }
    }

    fn get_key_handler(&self) -> &Self::KeyActions {
        &self.key_handler
    }

    fn get_key_handler_mut(&mut self) -> &mut Self::KeyActions {
        &mut self.key_handler
    }

    fn is_sim(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod test {
    use std::{env, error::Error};

    use devicemapper::{Bytes, Sectors};

    use crate::engine::{
        engine::Pool,
        strat_engine::{
            backstore::crypt_metadata_size,
            cmd,
            tests::{crypt, dm_stratis_devices_remove, loopbacked, real, FailDevice},
        },
        types::{ActionAvailability, EngineAction, KeyDescription},
    };

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

        let fs_name1 = "testfs1";
        let fs_name2 = "testfs2";
        let (_, pool) = engine.pools.get_mut_by_uuid(uuid1).unwrap();
        let fs_uuid1 = pool
            .create_filesystems(name1, uuid1, &[(fs_name1, None)])
            .unwrap()
            .changed()
            .unwrap();
        let fs_uuid2 = pool
            .create_filesystems(name1, uuid1, &[(fs_name2, None)])
            .unwrap()
            .changed()
            .unwrap();

        cmd::udev_settle().unwrap();

        assert!(Path::new(&format!("/dev/stratis/{}/{}", name1, fs_name1)).exists());
        assert!(Path::new(&format!("/dev/stratis/{}/{}", name1, fs_name2)).exists());

        let name2 = "name2";
        let action = engine.rename_pool(uuid1, name2).unwrap();

        cmd::udev_settle().unwrap();

        assert!(!Path::new(&format!("/dev/stratis/{}/{}", name1, fs_name1)).exists());
        assert!(!Path::new(&format!("/dev/stratis/{}/{}", name1, fs_name2)).exists());
        assert!(Path::new(&format!("/dev/stratis/{}/{}", name2, fs_name1)).exists());
        assert!(Path::new(&format!("/dev/stratis/{}/{}", name2, fs_name2)).exists());

        let (_, pool) = engine.pools.get_mut_by_uuid(uuid1).unwrap();
        pool.destroy_filesystems(
            name2,
            fs_uuid1
                .into_iter()
                .map(|(_, u, _)| u)
                .collect::<Vec<_>>()
                .as_slice(),
        )
        .unwrap();
        pool.destroy_filesystems(
            name2,
            fs_uuid2
                .into_iter()
                .map(|(_, u, _)| u)
                .collect::<Vec<_>>()
                .as_slice(),
        )
        .unwrap();

        assert_eq!(action, RenameAction::Renamed(uuid1));
        engine.teardown().unwrap();

        let engine = StratEngine::initialize().unwrap();
        let pool_name: String = engine.get_pool(uuid1).unwrap().0.to_owned();
        assert_eq!(pool_name, name2);
    }

    #[test]
    fn loop_test_pool_rename() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, Some(Sectors(10 * 1024 * 1024))),
            test_pool_rename,
        );
    }

    #[test]
    fn real_test_pool_rename() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(1, Some(Sectors(10 * 1024 * 1024)), None),
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

    fn create_pool_and_test_rollback<F>(
        name: &str,
        paths_with_fail_device: &[&Path],
        fail_device: &FailDevice,
        encryption_info: &EncryptionInfo,
        operation: F,
        unlock_method: UnlockMethod,
    ) -> Result<(), Box<dyn Error>>
    where
        F: Fn(&mut StratPool) -> Result<(), Box<dyn Error>>,
    {
        let mut engine = StratEngine::initialize()?;

        let uuid = engine
            .create_pool(name, paths_with_fail_device, None, Some(encryption_info))?
            .changed()
            .ok_or_else(|| {
                Box::new(StratisError::Msg(
                    "Pool should be newly created".to_string(),
                ))
            })?;
        let (_, pool) = engine
            .get_mut_pool(uuid)
            .ok_or_else(|| Box::new(StratisError::Msg("Pool must be present".to_string())))?;

        fail_device.start_failing(*Bytes(u128::from(crypt_metadata_size())).sectors())?;
        if operation(pool).is_ok() {
            return Err(Box::new(StratisError::Msg(
                "Clevis initialization should have failed".to_string(),
            )));
        }

        if pool.avail_actions() != ActionAvailability::Full {
            return Err(Box::new(StratisError::Msg(
                "Pool should have rolled back the change entirely".to_string(),
            )));
        }

        fail_device.stop_failing()?;

        engine.teardown()?;
        dm_stratis_devices_remove()?;

        let mut engine = StratEngine::initialize()?;
        engine.unlock_pool(uuid, unlock_method)?;

        engine.destroy_pool(uuid)?;
        engine.teardown()?;

        Ok(())
    }

    /// Test the creation of a pool with Clevis bindings, keyring bind, and unlock
    /// after rollback.
    fn test_keyring_bind_rollback(paths: &[&Path]) {
        fn test(paths: &[&Path], key_desc: &KeyDescription) -> Result<(), Box<dyn Error>> {
            let mut paths_with_fail_device = paths.to_vec();
            let last_device = paths_with_fail_device.pop().ok_or_else(|| {
                StratisError::Msg("Test requires at least one device".to_string())
            })?;
            let fail_device = FailDevice::new(last_device, "stratis_fail_device")?;
            cmd::udev_settle()?;
            let fail_device_path = fail_device.as_path();
            paths_with_fail_device.push(&fail_device_path);

            let name = "pool";
            let tang_url = env::var("TANG_URL")?;

            create_pool_and_test_rollback(
                name,
                paths_with_fail_device.as_slice(),
                &fail_device,
                &EncryptionInfo::ClevisInfo((
                    "tang".to_string(),
                    json!({
                        "url": tang_url,
                        "stratis:tang:trust_url": true
                    }),
                )),
                |pool| {
                    pool.bind_keyring(key_desc)?;
                    Ok(())
                },
                UnlockMethod::Clevis,
            )?;

            Ok(())
        }

        crypt::insert_and_cleanup_key(paths, test)
    }

    #[test]
    fn clevis_loop_test_keyring_bind_rollback() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_keyring_bind_rollback,
        );
    }

    #[test]
    fn clevis_real_test_keyring_bind_rollback() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_keyring_bind_rollback,
        );
    }

    /// Test the creation of a pool with a passphrase, keyring rebind, and unlock
    /// after rollback.
    fn test_keyring_rebind_rollback(paths: &[&Path]) {
        fn test(
            paths: &[&Path],
            key_desc1: &KeyDescription,
            key_desc2: &KeyDescription,
        ) -> Result<(), Box<dyn Error>> {
            let mut paths_with_fail_device = paths.to_vec();
            let last_device = paths_with_fail_device.pop().ok_or_else(|| {
                StratisError::Msg("Test requires at least one device".to_string())
            })?;
            let fail_device = FailDevice::new(last_device, "stratis_fail_device")?;
            cmd::udev_settle()?;
            let fail_device_path = fail_device.as_path();
            paths_with_fail_device.push(&fail_device_path);

            let name = "pool";

            create_pool_and_test_rollback(
                name,
                paths_with_fail_device.as_slice(),
                &fail_device,
                &EncryptionInfo::KeyDesc(key_desc1.to_owned()),
                |pool| {
                    pool.rebind_keyring(key_desc2)?;
                    // Change the key to ensure that the second key description
                    // is not the one that causes it to unlock successfully.
                    crypt::change_key(key_desc2)?;
                    Ok(())
                },
                UnlockMethod::Keyring,
            )?;

            Ok(())
        }

        crypt::insert_and_cleanup_two_keys(paths, test)
    }

    #[test]
    fn loop_test_keyring_rebind_rollback() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_keyring_rebind_rollback,
        );
    }

    #[test]
    fn real_test_keyring_rebind_rollback() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_keyring_rebind_rollback,
        );
    }

    /// Test the creation of a pool with a passphrase and Clevis bindings,
    /// keyring unbind, and unlock after rollback.
    fn test_keyring_unbind_rollback(paths: &[&Path]) {
        fn test(paths: &[&Path], key_desc: &KeyDescription) -> Result<(), Box<dyn Error>> {
            let mut paths_with_fail_device = paths.to_vec();
            let last_device = paths_with_fail_device.pop().ok_or_else(|| {
                StratisError::Msg("Test requires at least one device".to_string())
            })?;
            let fail_device = FailDevice::new(last_device, "stratis_fail_device")?;
            cmd::udev_settle()?;
            let fail_device_path = fail_device.as_path();
            paths_with_fail_device.push(&fail_device_path);

            let name = "pool";
            let tang_url = env::var("TANG_URL")?;

            create_pool_and_test_rollback(
                name,
                paths_with_fail_device.as_slice(),
                &fail_device,
                &EncryptionInfo::Both(
                    key_desc.to_owned(),
                    (
                        "tang".to_string(),
                        json!({
                            "url": tang_url,
                            "stratis:tang:trust_url": true
                        }),
                    ),
                ),
                |pool| {
                    pool.unbind_keyring()?;
                    Ok(())
                },
                UnlockMethod::Keyring,
            )?;

            Ok(())
        }

        crypt::insert_and_cleanup_key(paths, test)
    }

    #[test]
    fn clevis_loop_test_keyring_unbind_rollback() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_keyring_unbind_rollback,
        );
    }

    #[test]
    fn clevis_real_test_keyring_unbind_rollback() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_keyring_unbind_rollback,
        );
    }

    /// Test the creation of a pool with a passphrase, Clevis bind, and unlock
    /// after rollback.
    fn test_clevis_bind_rollback(paths: &[&Path]) {
        fn test(paths: &[&Path], key_desc: &KeyDescription) -> Result<(), Box<dyn Error>> {
            let mut paths_with_fail_device = paths.to_vec();
            let last_device = paths_with_fail_device.pop().ok_or_else(|| {
                StratisError::Msg("Test requires at least one device".to_string())
            })?;
            let fail_device = FailDevice::new(last_device, "stratis_fail_device")?;
            cmd::udev_settle()?;
            let fail_device_path = fail_device.as_path();
            paths_with_fail_device.push(&fail_device_path);

            let name = "pool";
            let tang_url = env::var("TANG_URL")?;

            create_pool_and_test_rollback(
                name,
                paths_with_fail_device.as_slice(),
                &fail_device,
                &EncryptionInfo::KeyDesc(key_desc.to_owned()),
                |pool| {
                    pool.bind_clevis(
                        "tang",
                        &json!({
                            "url": tang_url,
                            "stratis:tang:trust_url": true
                        }),
                    )?;
                    Ok(())
                },
                UnlockMethod::Keyring,
            )?;

            Ok(())
        }

        crypt::insert_and_cleanup_key(paths, test)
    }

    #[test]
    fn clevis_loop_test_clevis_bind_rollback() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_clevis_bind_rollback,
        );
    }

    #[test]
    fn clevis_real_test_clevis_bind_rollback() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_clevis_bind_rollback,
        );
    }

    /// Test the creation of a pool, Clevis rebind, and unlock after rollback.
    fn test_clevis_rebind_rollback(paths: &[&Path]) {
        let mut paths_with_fail_device = paths.to_vec();
        let last_device = paths_with_fail_device.pop().unwrap();
        let fail_device = FailDevice::new(last_device, "stratis_fail_device").unwrap();
        cmd::udev_settle().unwrap();
        let fail_device_path = fail_device.as_path();
        paths_with_fail_device.push(&fail_device_path);

        let name = "pool";
        let tang_url = env::var("TANG_URL").unwrap();

        create_pool_and_test_rollback(
            name,
            paths_with_fail_device.as_slice(),
            &fail_device,
            &EncryptionInfo::ClevisInfo((
                "tang".to_string(),
                json!({
                    "url": tang_url,
                    "stratis:tang:trust_url": true
                }),
            )),
            |pool| {
                pool.rebind_clevis()?;
                Ok(())
            },
            UnlockMethod::Clevis,
        )
        .unwrap();
    }

    #[test]
    fn clevis_loop_test_clevis_rebind_rollback() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_clevis_rebind_rollback,
        );
    }

    #[test]
    fn clevis_real_test_clevis_rebind_rollback() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_clevis_rebind_rollback,
        );
    }

    /// Test the creation of a pool with a passphrase and Clevis bindings,
    /// Clevis unbind, and unlock after rollback.
    fn test_clevis_unbind_rollback(paths: &[&Path]) {
        fn test(paths: &[&Path], key_desc: &KeyDescription) -> Result<(), Box<dyn Error>> {
            let mut paths_with_fail_device = paths.to_vec();
            let last_device = paths_with_fail_device.pop().ok_or_else(|| {
                StratisError::Msg("Test requires at least one device".to_string())
            })?;
            let fail_device = FailDevice::new(last_device, "stratis_fail_device")?;
            cmd::udev_settle()?;
            let fail_device_path = fail_device.as_path();
            paths_with_fail_device.push(&fail_device_path);

            let name = "pool";
            let tang_url = env::var("TANG_URL")?;

            create_pool_and_test_rollback(
                name,
                paths_with_fail_device.as_slice(),
                &fail_device,
                &EncryptionInfo::Both(
                    key_desc.to_owned(),
                    (
                        "tang".to_string(),
                        json!({
                            "url": tang_url,
                            "stratis:tang:trust_url": true
                        }),
                    ),
                ),
                |pool| {
                    pool.unbind_clevis()?;
                    Ok(())
                },
                UnlockMethod::Clevis,
            )?;

            Ok(())
        }

        crypt::insert_and_cleanup_key(paths, test)
    }

    #[test]
    fn clevis_loop_test_clevis_unbind_rollback() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_clevis_unbind_rollback,
        );
    }

    #[test]
    fn clevis_real_test_clevis_unbind_rollback() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_clevis_unbind_rollback,
        );
    }
}

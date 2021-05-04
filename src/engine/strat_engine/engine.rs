// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{clone::Clone, collections::HashMap, path::Path};

use async_trait::async_trait;
use serde_json::Value;

use devicemapper::DmNameBuf;

use crate::{
    engine::{
        engine::KeyActions,
        shared::{create_pool_idempotent_or_err, validate_name, validate_paths},
        strat_engine::{
            cmd::verify_binaries,
            dm::get_dm,
            keys::{MemoryFilesystem, StratKeyActions},
            liminal::{find_all, LiminalDevices},
            pool::StratPool,
        },
        structures::{Lockable, Table},
        types::{
            CreateAction, DeleteAction, DevUuid, EncryptionInfo, LockedPoolInfo, RenameAction,
            ReportType, SetUnlockAction, UdevEngineEvent, UnlockMethod,
        },
        Engine, Name, Pool, PoolUuid, Report,
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

#[derive(Debug)]
pub struct StratEngine {
    pools: Lockable<Table<PoolUuid, Lockable<StratPool>>>,

    // Maps pool UUIDs to information about sets of devices that are
    // associated with that UUID but have not been converted into a pool.
    liminal_devices: Lockable<LiminalDevices>,

    // Maps name of DM devices we are watching to the most recent event number
    // we've handled for each
    watched_dev_last_event_nrs: Lockable<HashMap<PoolUuid, HashMap<DmNameBuf, u32>>>,

    // Handler for key operations
    key_handler: Lockable<StratKeyActions>,

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
            pools.insert(pool_name, pool_uuid, Lockable::new(pool));
        }

        Ok(StratEngine {
            pools: Lockable::new(pools),
            liminal_devices: Lockable::new(liminal_devices),
            watched_dev_last_event_nrs: Lockable::new(HashMap::new()),
            key_handler: Lockable::new(StratKeyActions),
            key_fs: MemoryFilesystem::new()?,
        })
    }

    /// Recursively remove all devicemapper devices in all pools.
    /// Do not remove the dm-crypt devices that comprise the backstore.
    #[cfg(test)]
    pub fn teardown(self) -> StratisResult<()> {
        let mut untorndown_pools = Vec::new();
        let pools_lock = lock!(self.pools, read);
        for (_, uuid, pool) in pools_lock.iter() {
            lock!(pool, write)
                .teardown()
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

#[async_trait]
impl Report for StratEngine {
    // Precondition: (&StratPool).into() pattern matches Value::Object(_)
    // Precondition: (&LiminalDevices).into() pattern matches Value::Object(_)
    async fn engine_state_report(&self) -> Value {
        let json = json!({
            "pools": Value::Array({
                let mut pools = Vec::new();
                for (name, uuid, pool) in self.pools.read().await.iter() {
                    let mut json = json!({
                        "uuid": Value::from(uuid.to_string()),
                        "name": Value::from(name.to_string()),
                    });
                    if let Value::Object(ref mut map) = json {
                        map.extend(
                            if let Value::Object(map) = <&StratPool as Into<Value>>::into(&*pool.read().await) {
                                map.into_iter()
                            } else {
                                unreachable!("StratPool conversion returns a JSON object");
                            }
                        );
                    } else {
                        unreachable!("json!() always creates a JSON object")
                    }
                    pools.push(json);
                }
                pools
            }),
        });
        if let (Value::Object(mut j), Value::Object(map)) = (
            json,
            <&LiminalDevices as Into<Value>>::into(&*lock!(self.liminal_devices, read)),
        ) {
            j.extend(map.into_iter());
            Value::Object(j)
        } else {
            unreachable!("json!() and LiminalDevices::into() always return JSON object");
        }
    }

    async fn get_report(&self, report_type: ReportType) -> Value {
        match report_type {
            ReportType::ErroredPoolDevices => (&*self.liminal_devices.read().await).into(),
        }
    }
}

#[async_trait]
impl Engine for StratEngine {
    async fn handle_event(
        &self,
        event: &UdevEngineEvent,
    ) -> Option<(Name, PoolUuid, Lockable<dyn Pool>)> {
        let pools_lock = self.pools.read().await;
        let pool_info = self
            .liminal_devices
            .write()
            .await
            .block_evaluate(&*pools_lock, event);
        if let Some((pool_uuid, pool_name, pool)) = pool_info {
            self.pools
                .write()
                .await
                .insert(pool_name.clone(), pool_uuid, Lockable::new(pool));
            Some((
                pool_name,
                pool_uuid,
                self.pools
                    .read()
                    .await
                    .get_by_uuid(pool_uuid)
                    .expect("just_inserted")
                    .1
                    .clone()
                    .into_dyn_pool(),
            ))
        } else {
            None
        }
    }

    async fn create_pool(
        &self,
        name: &str,
        blockdev_paths: &[&Path],
        redundancy: Option<u16>,
        encryption_info: &EncryptionInfo,
    ) -> StratisResult<CreateAction<PoolUuid>> {
        let redundancy = calculate_redundancy!(redundancy);

        validate_name(name)?;

        validate_paths(blockdev_paths)?;

        let pool = self
            .pools
            .read()
            .await
            .get_by_name(name)
            .map(|(_, p)| p.clone());
        if let Some(pool) = pool {
            create_pool_idempotent_or_err(pool.into_dyn_pool(), name, blockdev_paths).await
        } else if blockdev_paths.is_empty() {
            Err(StratisError::Engine(
                ErrorEnum::Invalid,
                "At least one blockdev is required to create a pool.".to_string(),
            ))
        } else {
            let (uuid, pool) =
                StratPool::initialize(name, blockdev_paths, redundancy, encryption_info)?;

            let name = Name::new(name.to_owned());
            self.pools
                .write()
                .await
                .insert(name, uuid, Lockable::new(pool));
            Ok(CreateAction::Created(uuid))
        }
    }

    async fn destroy_pool(&self, uuid: PoolUuid) -> StratisResult<DeleteAction<PoolUuid>> {
        if let Some((_, pool)) = self.pools.read().await.get_by_uuid(uuid) {
            if pool.read().await.has_filesystems() {
                return Err(StratisError::Engine(
                    ErrorEnum::Busy,
                    "filesystems remaining on pool".into(),
                ));
            };
        } else {
            return Ok(DeleteAction::Identity);
        }

        let mut pool_lock = self.pools.write().await;
        let (pool_name, pool) = pool_lock
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pools.get_by_uuid() returned a value");

        let pool_clone = pool.clone();
        let pool_res = spawn_blocking!(lock!(pool_clone, write).destroy());
        if let Err(err) = pool_res {
            pool_lock.insert(pool_name, uuid, pool);
            Err(err)
        } else {
            Ok(DeleteAction::Deleted(uuid))
        }
    }

    async fn rename_pool(
        &self,
        uuid: PoolUuid,
        new_name: &str,
    ) -> StratisResult<RenameAction<PoolUuid>> {
        validate_name(new_name)?;
        let old_name = rename_pool_pre_idem!(self; uuid; new_name);

        let mut pool_lock = self.pools.write().await;
        let (_, pool) = pool_lock
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pools.get_by_uuid() returned a value");
        let pool_clone = pool.clone();

        let new_name = Name::new(new_name.to_owned());
        let new_name_clone = new_name.clone();
        let pool_res = spawn_blocking!(lock!(pool_clone, write).write_metadata(&new_name_clone));
        if let Err(err) = pool_res {
            pool_lock.insert(old_name, uuid, pool);
            Err(err)
        } else {
            pool_lock.insert(new_name.clone(), uuid, pool.clone());
            drop(pool_lock);

            pool.read().await.udev_pool_change(&new_name);
            Ok(RenameAction::Renamed(uuid))
        }
    }

    async fn unlock_pool(
        &self,
        pool_uuid: PoolUuid,
        unlock_method: UnlockMethod,
    ) -> StratisResult<SetUnlockAction<DevUuid>> {
        let liminal_devices = self.liminal_devices.clone();
        let pools = self.pools.clone();
        let unlocked = spawn_blocking!(lock!(liminal_devices, write).unlock_pool(
            &*lock!(pools, read),
            pool_uuid,
            unlock_method,
        ))?;
        Ok(SetUnlockAction::new(unlocked))
    }

    async fn get_pool(&self, uuid: PoolUuid) -> Option<(Name, Lockable<dyn Pool>)> {
        get_pool!(self; uuid)
    }

    async fn locked_pools(&self) -> HashMap<PoolUuid, LockedPoolInfo> {
        self.liminal_devices.read().await.locked_pools()
    }

    async fn pools(&self) -> Vec<(Name, PoolUuid, Lockable<dyn Pool>)> {
        self.pools
            .read()
            .await
            .iter()
            .map(|(name, uuid, pool)| (name.clone(), *uuid, pool.clone().into_dyn_pool()))
            .collect()
    }

    async fn evented(&self) -> StratisResult<()> {
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

        let pool_lock = self.pools.read().await;
        for (pool_name, pool_uuid, pool) in pool_lock.iter() {
            let dev_names = pool.read().await.get_eventing_dev_names(*pool_uuid);
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

            if self.watched_dev_last_event_nrs.read().await.get(pool_uuid) != Some(&event_nrs) {
                // Return error early before updating the watched event numbers
                // so that if another event comes in on any pool, this method
                // will retry eventing as the event number will be higher than
                // what was previously recorded.
                let pool = pool.clone();
                let pool_uuid = *pool_uuid;
                let pool_name = pool_name.clone();
                spawn_blocking!(lock!(pool, write).event_on(pool_uuid, &pool_name))?;
            }
            self.watched_dev_last_event_nrs
                .write()
                .await
                .insert(*pool_uuid, event_nrs);
        }

        Ok(())
    }

    fn get_key_handler(&self) -> Lockable<dyn KeyActions> {
        self.key_handler.clone().into_dyn_key_handler()
    }

    fn is_sim(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod test {
    use devicemapper::Sectors;
    use futures::executor::block_on;

    use crate::engine::{
        strat_engine::{
            cmd,
            tests::{loopbacked, real},
        },
        types::EngineAction,
    };

    use super::*;

    /// Verify that a pool rename causes the pool metadata to get the new name.
    fn test_pool_rename(paths: &[&Path]) {
        let engine = StratEngine::initialize().unwrap();

        let name1 = "name1";
        let uuid1 = block_on(engine.create_pool(name1, paths, None, &EncryptionInfo::default()))
            .unwrap()
            .changed()
            .unwrap();

        let fs_name1 = "testfs1";
        let fs_name2 = "testfs2";
        let (_, pool) = block_on(engine.get_pool(uuid1)).unwrap();
        let fs_uuid1 = lock!(pool, write)
            .create_filesystems(name1, uuid1, &[(fs_name1, None)])
            .unwrap()
            .changed()
            .unwrap();
        let fs_uuid2 = lock!(pool, write)
            .create_filesystems(name1, uuid1, &[(fs_name2, None)])
            .unwrap()
            .changed()
            .unwrap();

        cmd::udev_settle().unwrap();

        assert!(Path::new(&format!("/dev/stratis/{}/{}", name1, fs_name1)).exists());
        assert!(Path::new(&format!("/dev/stratis/{}/{}", name1, fs_name2)).exists());

        let name2 = "name2";
        let action = block_on!(engine.rename_pool(uuid1, name2)).unwrap();

        cmd::udev_settle().unwrap();

        assert!(!Path::new(&format!("/dev/stratis/{}/{}", name1, fs_name1)).exists());
        assert!(!Path::new(&format!("/dev/stratis/{}/{}", name1, fs_name2)).exists());
        assert!(Path::new(&format!("/dev/stratis/{}/{}", name2, fs_name1)).exists());
        assert!(Path::new(&format!("/dev/stratis/{}/{}", name2, fs_name2)).exists());

        let (_, pool) = block_on(engine.get_pool(uuid1)).unwrap();
        lock!(pool, write)
            .destroy_filesystems(
                name2,
                fs_uuid1
                    .into_iter()
                    .map(|(_, u)| u)
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
            .unwrap();
        lock!(pool, write)
            .destroy_filesystems(
                name2,
                fs_uuid2
                    .into_iter()
                    .map(|(_, u)| u)
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
            .unwrap();

        assert_eq!(action, RenameAction::Renamed(uuid1));
        engine.teardown().unwrap();

        let engine = StratEngine::initialize().unwrap();
        let pool_name: String = block_on(engine.get_pool(uuid1)).unwrap().0.to_owned();
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

        let engine = StratEngine::initialize().unwrap();

        let name1 = "name1";
        let uuid1 = block_on(engine.create_pool(name1, paths1, None, &EncryptionInfo::default()))
            .unwrap()
            .changed()
            .unwrap();

        let name2 = "name2";
        let uuid2 = block_on(engine.create_pool(name2, paths2, None, &EncryptionInfo::default()))
            .unwrap()
            .changed()
            .unwrap();

        assert!(block_on(engine.get_pool(uuid1)).is_some());
        assert!(block_on(engine.get_pool(uuid2)).is_some());

        engine.teardown().unwrap();

        let engine = StratEngine::initialize().unwrap();

        assert!(block_on(engine.get_pool(uuid1)).is_some());
        assert!(block_on(engine.get_pool(uuid2)).is_some());

        engine.teardown().unwrap();

        let engine = StratEngine::initialize().unwrap();

        assert!(block_on(engine.get_pool(uuid1)).is_some());
        assert!(block_on(engine.get_pool(uuid2)).is_some());

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

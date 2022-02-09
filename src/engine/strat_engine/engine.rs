// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{HashMap, HashSet},
    convert::TryFrom,
    path::Path,
    sync::Arc,
};

use async_trait::async_trait;
use futures::{executor::block_on, future::join_all};
use serde_json::Value;
use tokio::{
    sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock},
    task::{spawn_blocking, JoinHandle},
};

use devicemapper::DmNameBuf;

use crate::{
    engine::{
        shared::{create_pool_idempotent_or_err, validate_name, validate_paths},
        strat_engine::{
            backstore::ProcessedPaths,
            cmd::verify_binaries,
            dm::get_dm,
            keys::{MemoryFilesystem, StratKeyActions},
            liminal::{find_all, LiminalDevices},
            pool::StratPool,
        },
        structures::{
            AllLockReadGuard, AllLockWriteGuard, AllOrSomeLock, ExclusiveGuard, Lockable,
            SharedGuard, SomeLockReadGuard, SomeLockWriteGuard, Table,
        },
        types::{
            CreateAction, DeleteAction, DevUuid, EncryptionInfo, FilesystemUuid, LockKey,
            LockedPoolInfo, PoolDiff, RenameAction, ReportType, SetUnlockAction,
            StratFilesystemDiff, UdevEngineEvent, UnlockMethod,
        },
        Engine, Name, PoolUuid, Report,
    },
    stratis::{StratisError, StratisResult},
};

type EventNumbers = HashMap<PoolUuid, HashMap<DmNameBuf, u32>>;
type PoolJoinHandles = Vec<JoinHandle<StratisResult<(PoolUuid, PoolDiff)>>>;

#[derive(Debug)]
pub struct StratEngine {
    pools: AllOrSomeLock<PoolUuid, StratPool>,

    // Maps pool UUIDs to information about sets of devices that are
    // associated with that UUID but have not been converted into a pool.
    liminal_devices: Lockable<Arc<RwLock<LiminalDevices>>>,

    // Maps name of DM devices we are watching to the most recent event number
    // we've handled for each
    watched_dev_last_event_nrs: Lockable<Arc<RwLock<EventNumbers>>>,

    // Handler for key operations
    key_handler: Lockable<Arc<RwLock<StratKeyActions>>>,

    // TODO: Remove this code when Clevis supports reading keys from the
    // kernel keyring.
    // In memory filesystem for passing keys to Clevis.
    // See GitHub issue: https://github.com/stratis-storage/project/issues/212.
    #[allow(dead_code)]
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
            pools: AllOrSomeLock::new(pools),
            liminal_devices: Lockable::new_shared(liminal_devices),
            watched_dev_last_event_nrs: Lockable::new_shared(HashMap::new()),
            key_handler: Lockable::new_shared(StratKeyActions),
            key_fs: MemoryFilesystem::new()?,
        })
    }

    fn spawn_pool_check_handling(
        joins: &mut PoolJoinHandles,
        guard: SomeLockWriteGuard<PoolUuid, StratPool>,
    ) {
        joins.push(spawn_blocking(move || {
            let (name, uuid, pool) = guard.as_tuple();
            Ok((uuid, pool.event_on(uuid, &name)?))
        }));
    }

    fn spawn_fs_check_handling(
        joins: &mut Vec<JoinHandle<StratisResult<HashMap<FilesystemUuid, StratFilesystemDiff>>>>,
        guard: SomeLockWriteGuard<PoolUuid, StratPool>,
    ) {
        joins.push(spawn_blocking(move || {
            let (name, uuid, pool) = guard.as_tuple();
            pool.fs_event_on(uuid, &name)
        }));
    }

    async fn join_all_pool_checks(handles: PoolJoinHandles) -> HashMap<PoolUuid, PoolDiff> {
        join_all(handles)
            .await
            .into_iter()
            .filter_map(|res| match res {
                Ok(Ok(tup)) => Some(tup),
                Ok(Err(e)) => {
                    warn!("Pool checks failed with error: {}", e);
                    None
                }
                Err(e) => {
                    warn!(
                        "Failed to get status for thread handling pool checks: {}",
                        e
                    );
                    None
                }
            })
            .collect::<HashMap<_, _>>()
    }

    async fn join_all_fs_checks(
        handles: Vec<JoinHandle<StratisResult<HashMap<FilesystemUuid, StratFilesystemDiff>>>>,
    ) -> HashMap<FilesystemUuid, StratFilesystemDiff> {
        join_all(handles)
            .await
            .into_iter()
            .fold(HashMap::default(), |mut acc, next| match next {
                Ok(Ok(hm)) => {
                    acc.extend(hm);
                    acc
                }
                Ok(Err(e)) => {
                    warn!("Filesystem checks failed with error: {}", e);
                    acc
                }
                Err(e) => {
                    warn!(
                        "Failed to get status for thread handling filesystem checks: {}",
                        e
                    );
                    acc
                }
            })
    }

    /// The implementation for pool_evented when caused by a devicemapper event.
    async fn pool_evented_dm(&self, pools: &HashSet<PoolUuid>) -> HashMap<PoolUuid, PoolDiff> {
        let mut joins = Vec::new();
        for uuid in pools {
            if let Some(guard) = self.pools.write(LockKey::Uuid(*uuid)).await {
                Self::spawn_pool_check_handling(&mut joins, guard);
            } else {
                warn!(
                    "Pool with UUID {} indicated an event but could not be found when attempting to handle the event; it may have been deleted during the time between detection and handling",
                    uuid
                );
            }
        }

        Self::join_all_pool_checks(joins).await
    }

    /// The implementation for pool_evented when called by the timer thread.
    async fn pool_evented_timer(&self) -> HashMap<PoolUuid, PoolDiff> {
        let mut joins = Vec::new();
        let guards: Vec<SomeLockWriteGuard<PoolUuid, StratPool>> =
            self.pools.write_all().await.into();
        for guard in guards {
            Self::spawn_pool_check_handling(&mut joins, guard);
        }

        Self::join_all_pool_checks(joins).await
    }

    /// The implementation for fs_evented when caused by a devicemapper event.
    async fn fs_evented_dm(
        &self,
        pools: &HashSet<PoolUuid>,
    ) -> HashMap<FilesystemUuid, StratFilesystemDiff> {
        let mut joins = Vec::new();
        for uuid in pools {
            if let Some(guard) = self.pools.write(LockKey::Uuid(*uuid)).await {
                Self::spawn_fs_check_handling(&mut joins, guard);
            } else {
                warn!(
                    "Pool with UUID {} indicated an event but could not be found when attempting to handle the event; it may have been deleted during the time between detection and handling",
                    uuid
                );
            }
        }

        Self::join_all_fs_checks(joins).await
    }

    /// The implementation for fs_evented when called by the timer thread.
    async fn fs_evented_timer(&self) -> HashMap<FilesystemUuid, StratFilesystemDiff> {
        let mut joins = Vec::new();
        let guards: Vec<SomeLockWriteGuard<PoolUuid, StratPool>> =
            self.pools.write_all().await.into();
        for guard in guards {
            Self::spawn_fs_check_handling(&mut joins, guard);
        }

        Self::join_all_fs_checks(joins).await
    }

    /// Recursively remove all devicemapper devices in all pools.
    /// Do not remove the dm-crypt devices that comprise the backstore.
    #[cfg(test)]
    pub fn teardown(self) -> StratisResult<()> {
        let mut untorndown_pools = Vec::new();
        let mut write_all = block_on(self.pools.write_all());
        for (_, uuid, pool) in write_all.iter_mut() {
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
                block_on(self.pools.read_all()).iter()
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
            <&LiminalDevices as Into<Value>>::into(&*self.liminal_devices.blocking_read()),
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
            ReportType::ErroredPoolDevices => (&*self.liminal_devices.blocking_read()).into(),
        }
    }
}

#[async_trait]
impl Engine for StratEngine {
    type Pool = StratPool;
    type KeyActions = StratKeyActions;

    async fn handle_events(
        &self,
        events: Vec<UdevEngineEvent>,
    ) -> Vec<SomeLockReadGuard<PoolUuid, Self::Pool>> {
        let mut ret_guards = Vec::new();
        let uuids = {
            let mut ld_guard = self.liminal_devices.write().await;
            let mut pools_write_all = self.pools.write_all().await;
            match spawn_blocking!({
                events
                    .into_iter()
                    .filter_map(|event| {
                        if let Some((uuid, name, pool)) =
                            ld_guard.block_evaluate(&*pools_write_all, &event)
                        {
                            pools_write_all.insert(name, uuid, pool);
                            Some(uuid)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            }) {
                Ok(u) => u,
                Err(e) => {
                    warn!("Failed to handle udev events: {}", e);
                    return ret_guards;
                }
            }
        };
        for uuid in uuids {
            if let Some(guard) = self.pools.read(LockKey::Uuid(uuid)).await {
                ret_guards.push(guard);
            }
        }
        ret_guards
    }

    async fn create_pool(
        &self,
        name: &str,
        blockdev_paths: &[&Path],
        encryption_info: Option<&EncryptionInfo>,
    ) -> StratisResult<CreateAction<PoolUuid>> {
        validate_name(name)?;
        let name = Name::new(name.to_owned());

        validate_paths(blockdev_paths)?;

        let maybe_guard = self.pools.read(LockKey::Name(name.clone())).await;
        if let Some(guard) = maybe_guard {
            let (name, _, pool) = guard.as_tuple();
            create_pool_idempotent_or_err(pool, &name, blockdev_paths)
        } else if blockdev_paths.is_empty() {
            Err(StratisError::Msg(
                "At least one blockdev is required to create a pool.".to_string(),
            ))
        } else {
            let cloned_paths = blockdev_paths
                .iter()
                .map(|p| p.to_path_buf())
                .collect::<Vec<_>>();

            let device_infos = spawn_blocking!({
                let borrowed_paths = cloned_paths.iter().map(|p| p.as_path()).collect::<Vec<_>>();
                ProcessedPaths::try_from(borrowed_paths.as_slice())
            })??;

            if device_infos.has_stratis_devices() {
                let mut error_message_entries = vec![];
                let pools_read_all = self.pools.read_all().await;
                for (pool_uuid, device_map) in device_infos.stratis_devices() {
                    if let Some(entry) = pools_read_all.get_by_uuid(*pool_uuid) {
                        for (_, dev_info) in device_map.iter() {
                            error_message_entries.push((
                                pool_uuid,
                                entry.0.to_owned(),
                                dev_info.devnode.to_owned(),
                            ));
                        }
                    }
                }
                let error_message = format!(
                    "The specified devices appear to already belong to some Stratis pool: {}",
                    error_message_entries
                        .iter()
                        .map(|(pool_name, pool_uuid, path)| format!(
                            "Path {} belongs to pool {} with UUID {}",
                            path.display(),
                            pool_name,
                            pool_uuid
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                return Err(StratisError::Msg(error_message));
            }

            let cloned_name = name.clone();
            let cloned_enc_info = encryption_info.cloned();

            let pool_uuid = PoolUuid::new_v4();
            let _ = spawn_blocking!({
                StratPool::initialize(
                    pool_uuid,
                    &cloned_name,
                    device_infos
                        .into_unowned()
                        .expect("returned an error if any stratis owned devices"),
                    cloned_enc_info.as_ref(),
                )
            })??;

            Ok(CreateAction::Created(pool_uuid))
        }
    }

    async fn destroy_pool(&self, uuid: PoolUuid) -> StratisResult<DeleteAction<PoolUuid>> {
        if let Some(pool) = self.pools.read(LockKey::Uuid(uuid)).await {
            if pool.has_filesystems() {
                return Err(StratisError::Msg("filesystems remaining on pool".into()));
            };
        } else {
            return Ok(DeleteAction::Identity);
        }

        let mut guard = self.pools.write_all().await;
        let (pool_name, mut pool) = guard
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pools.get_by_uuid() returned a value");

        let (res, pool) = spawn_blocking!((pool.destroy(), pool))?;
        if let Err(err) = res {
            guard.insert(pool_name, uuid, pool);
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
        let new_name = Name::new(new_name.to_owned());
        let old_name = rename_pool_pre_idem!(self; uuid; new_name.clone());

        let mut guard = self.pools.write_all().await;

        let (_, mut pool) = guard
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pools.get_by_uuid() returned a value");

        let cloned_new_name = new_name.clone();
        let (res, pool) = spawn_blocking!((pool.write_metadata(&cloned_new_name), pool))?;
        if let Err(err) = res {
            guard.insert(old_name, uuid, pool);
            Err(err)
        } else {
            guard.insert(new_name, uuid, pool);
            let (new_name, pool) = guard.get_by_uuid(uuid).expect("Inserted above");
            pool.udev_pool_change(&new_name);
            Ok(RenameAction::Renamed(uuid))
        }
    }

    async fn unlock_pool(
        &self,
        pool_uuid: PoolUuid,
        unlock_method: UnlockMethod,
    ) -> StratisResult<SetUnlockAction<DevUuid>> {
        let mut ld_guard = self.liminal_devices.write().await;
        let pools_read_all = self.pools.read_all().await;
        let unlocked =
            spawn_blocking!(ld_guard.unlock_pool(&*pools_read_all, pool_uuid, unlock_method,))??;
        Ok(SetUnlockAction::new(unlocked))
    }

    async fn get_pool(
        &self,
        key: LockKey<PoolUuid>,
    ) -> Option<SomeLockReadGuard<PoolUuid, Self::Pool>> {
        get_pool!(self; key)
    }

    async fn get_mut_pool(
        &self,
        key: LockKey<PoolUuid>,
    ) -> Option<SomeLockWriteGuard<PoolUuid, Self::Pool>> {
        get_mut_pool!(self; key)
    }

    async fn locked_pools(&self) -> HashMap<PoolUuid, LockedPoolInfo> {
        self.liminal_devices.read().await.locked_pools()
    }

    async fn pools(&self) -> AllLockReadGuard<PoolUuid, Self::Pool> {
        self.pools.read_all().await
    }

    async fn pools_mut(&self) -> AllLockWriteGuard<PoolUuid, Self::Pool> {
        self.pools.write_all().await
    }

    async fn get_events(&self) -> StratisResult<HashSet<PoolUuid>> {
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

        let mut changed = HashSet::new();
        let read_pools = self.pools.read_all().await;
        let mut write_event_nrs = self.watched_dev_last_event_nrs.write().await;
        for (_, pool_uuid, pool) in read_pools.iter() {
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

            if write_event_nrs.get(pool_uuid) != Some(&event_nrs) {
                changed.insert(*pool_uuid);
            }

            write_event_nrs.insert(*pool_uuid, event_nrs);
        }

        Ok(changed)
    }

    async fn pool_evented(&self, pools: Option<&HashSet<PoolUuid>>) -> HashMap<PoolUuid, PoolDiff> {
        match pools {
            Some(ps) => self.pool_evented_dm(ps).await,
            None => self.pool_evented_timer().await,
        }
    }

    async fn fs_evented(
        &self,
        pools: Option<&HashSet<PoolUuid>>,
    ) -> HashMap<FilesystemUuid, StratFilesystemDiff> {
        match pools {
            Some(ps) => self.fs_evented_dm(ps).await,
            None => self.fs_evented_timer().await,
        }
    }

    async fn get_key_handler(&self) -> SharedGuard<OwnedRwLockReadGuard<Self::KeyActions>> {
        self.key_handler.read().await
    }

    async fn get_key_handler_mut(&self) -> ExclusiveGuard<OwnedRwLockWriteGuard<Self::KeyActions>> {
        self.key_handler.write().await
    }

    fn is_sim(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod test {
    use std::{env, error::Error, ffi::OsStr, iter::once, path::Path};

    use devicemapper::{Bytes, Sectors};

    use crate::engine::{
        engine::Pool,
        strat_engine::{
            backstore::crypt_metadata_size,
            cmd,
            tests::{crypt, dm_stratis_devices_remove, loopbacked, real, FailDevice},
            udev::{CRYPTO_FS_TYPE, FS_TYPE_KEY},
        },
        types::{
            ActionAvailability, EngineAction, KeyDescription, UdevEngineDevice, UdevEngineEvent,
        },
    };

    use super::*;

    /// Verify that a pool rename causes the pool metadata to get the new name.
    fn test_pool_rename(paths: &[&Path]) {
        let engine = StratEngine::initialize().unwrap();

        let name1 = "name1";
        let uuid1 = test_async!(engine.create_pool(name1, paths, None))
            .unwrap()
            .changed()
            .unwrap();

        let events = generate_events!();
        test_async!(engine.handle_events(events));

        let fs_name1 = "testfs1";
        let fs_name2 = "testfs2";
        let (fs_uuid1, fs_uuid2) = {
            let mut pool = test_async!(engine.get_mut_pool(LockKey::Uuid(uuid1))).unwrap();
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
            (fs_uuid1, fs_uuid2)
        };

        cmd::udev_settle().unwrap();

        assert!(Path::new(&format!("/dev/stratis/{}/{}", name1, fs_name1)).exists());
        assert!(Path::new(&format!("/dev/stratis/{}/{}", name1, fs_name2)).exists());

        let name2 = "name2";
        let action = test_async!(engine.rename_pool(uuid1, name2)).unwrap();

        cmd::udev_settle().unwrap();

        assert!(!Path::new(&format!("/dev/stratis/{}/{}", name1, fs_name1)).exists());
        assert!(!Path::new(&format!("/dev/stratis/{}/{}", name1, fs_name2)).exists());
        assert!(Path::new(&format!("/dev/stratis/{}/{}", name2, fs_name1)).exists());
        assert!(Path::new(&format!("/dev/stratis/{}/{}", name2, fs_name2)).exists());

        {
            let mut pool = test_async!(engine.get_mut_pool(LockKey::Uuid(uuid1))).unwrap();
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
        }

        assert_eq!(action, RenameAction::Renamed(uuid1));
        engine.teardown().unwrap();

        let engine = StratEngine::initialize().unwrap();
        let pool_name: String = test_async!(engine.get_pool(LockKey::Uuid(uuid1)))
            .unwrap()
            .as_tuple()
            .0
            .to_owned();
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
        let uuid1 = test_async!(engine.create_pool(name1, paths1, None))
            .unwrap()
            .changed()
            .unwrap();

        let name2 = "name2";
        let uuid2 = test_async!(engine.create_pool(name2, paths2, None))
            .unwrap()
            .changed()
            .unwrap();

        let events = generate_events!();
        test_async!(engine.handle_events(events));

        assert!(test_async!(engine.get_pool(LockKey::Uuid(uuid1))).is_some());
        assert!(test_async!(engine.get_pool(LockKey::Uuid(uuid2))).is_some());

        engine.teardown().unwrap();

        let engine = StratEngine::initialize().unwrap();

        assert!(test_async!(engine.get_pool(LockKey::Uuid(uuid1))).is_some());
        assert!(test_async!(engine.get_pool(LockKey::Uuid(uuid2))).is_some());

        engine.teardown().unwrap();

        let engine = StratEngine::initialize().unwrap();

        assert!(test_async!(engine.get_pool(LockKey::Uuid(uuid1))).is_some());
        assert!(test_async!(engine.get_pool(LockKey::Uuid(uuid2))).is_some());

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
        fn needs_clean_up<F>(
            engine: StratEngine,
            uuid: PoolUuid,
            fail_device: &FailDevice,
            operation: F,
        ) -> Result<(), Box<dyn Error>>
        where
            F: Fn(&mut StratPool) -> Result<(), Box<dyn Error>>,
        {
            {
                let mut pool =
                    test_async!(engine.get_mut_pool(LockKey::Uuid(uuid))).ok_or_else(|| {
                        Box::new(StratisError::Msg("Pool must be present".to_string()))
                    })?;

                fail_device.start_failing(*Bytes(u128::from(crypt_metadata_size())).sectors())?;
                if operation(&mut pool).is_ok() {
                    return Err(Box::new(StratisError::Msg(
                        "Clevis initialization should have failed".to_string(),
                    )));
                }

                if pool.avail_actions() != ActionAvailability::Full {
                    return Err(Box::new(StratisError::Msg(
                        "Pool should have rolled back the change entirely".to_string(),
                    )));
                }
            }

            fail_device.stop_failing()?;

            engine.teardown()?;

            Ok(())
        }

        let engine = StratEngine::initialize()?;
        let uuid =
            test_async!(engine.create_pool(name, paths_with_fail_device, Some(encryption_info)))?
                .changed()
                .ok_or_else(|| {
                    Box::new(StratisError::Msg(
                        "Pool should be newly created".to_string(),
                    ))
                })?;

        let mut events = generate_events!();

        // Hack because fail device does not show up as LUKS2 device in udev
        let cxt = libudev::Context::new().expect("Creating udev context should succeed");
        let mut enumerator =
            libudev::Enumerator::new(&cxt).expect("Creating enumerator should succeed");
        enumerator.match_is_initialized().unwrap();
        let mut devices = enumerator
            .scan_devices()
            .expect("Scanning udev devices should succeed");

        let fail_udev = devices
            .find(|dev| dev.devnode() == Some(&fail_device.as_path().canonicalize().unwrap()))
            .expect("Fail device must be in udev database");

        events.push(UdevEngineEvent::new(
            libudev::EventType::Add,
            UdevEngineDevice::new(
                fail_udev.is_initialized(),
                fail_udev.devnode().map(|p| p.to_owned()),
                fail_udev.devnum(),
                fail_udev
                    .properties()
                    .map(|prop| (Box::from(prop.name()), Box::from(prop.value())))
                    .chain(once((
                        Box::from(OsStr::new(FS_TYPE_KEY)),
                        Box::from(OsStr::new(CRYPTO_FS_TYPE)),
                    )))
                    .collect::<HashMap<_, _>>(),
            ),
        ));

        test_async!(engine.handle_events(events));

        let res = needs_clean_up(engine, uuid, fail_device, operation);

        dm_stratis_devices_remove()?;
        res?;

        let engine = StratEngine::initialize()?;
        test_async!(engine.unlock_pool(uuid, unlock_method))?;
        test_async!(engine.destroy_pool(uuid))?;
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

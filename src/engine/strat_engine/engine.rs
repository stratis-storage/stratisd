// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{HashMap, HashSet},
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
        engine::HandleEvents,
        shared::{create_pool_idempotent_or_err, validate_name, validate_paths},
        strat_engine::{
            backstore::ProcessedPathInfos,
            cmd::verify_executables,
            dm::get_dm,
            keys::StratKeyActions,
            liminal::{find_all, LiminalDevices},
            ns::MemoryFilesystem,
            pool::StratPool,
        },
        structures::{
            AllLockReadGuard, AllLockWriteGuard, AllOrSomeLock, ExclusiveGuard, Lockable,
            SharedGuard, SomeLockReadGuard, SomeLockWriteGuard, Table,
        },
        types::{
            CreateAction, DeleteAction, DevUuid, EncryptionInfo, FilesystemUuid, LockKey,
            LockedPoolInfo, PoolDiff, RenameAction, ReportType, SetUnlockAction, StartAction,
            StopAction, StoppedPoolInfo, StratFilesystemDiff, UdevEngineEvent, UnlockMethod,
        },
        Engine, Name, Pool, PoolUuid, Report,
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

    // In memory filesystem for private namespace mounts.
    #[allow(dead_code)]
    fs: MemoryFilesystem,
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
    /// Returns an error if the executables on which it depends can not be found.
    pub fn initialize() -> StratisResult<StratEngine> {
        let fs = MemoryFilesystem::new()?;
        verify_executables()?;

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
            fs,
        })
    }

    fn spawn_pool_check_handling(
        joins: &mut PoolJoinHandles,
        mut guard: SomeLockWriteGuard<PoolUuid, StratPool>,
    ) {
        joins.push(spawn_blocking(move || {
            let (name, uuid, pool) = guard.as_mut_tuple();
            Ok((uuid, pool.event_on(uuid, &name)?))
        }));
    }

    fn spawn_fs_check_handling(
        joins: &mut Vec<JoinHandle<StratisResult<HashMap<FilesystemUuid, StratFilesystemDiff>>>>,
        mut guard: SomeLockWriteGuard<PoolUuid, StratPool>,
    ) {
        joins.push(spawn_blocking(move || {
            let (_, uuid, pool) = guard.as_mut_tuple();
            pool.fs_event_on(uuid)
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
            ReportType::StoppedPools => (&*self.liminal_devices.blocking_read()).into(),
        }
    }
}

#[async_trait]
impl Engine for StratEngine {
    type Pool = StratPool;
    type KeyActions = StratKeyActions;

    async fn handle_events(&self, events: Vec<UdevEngineEvent>) -> HandleEvents<Self::Pool> {
        let mut ret_guards = Vec::new();
        let mut diffs = HashMap::new();

        // Acquire a write lock here so that no concurrent accesses can occur between
        // udev event handling and insertion into the pool.
        //
        // Failing to do this can cause two identical pools to be registered in
        // internal data structures.
        let mut pools_write_all = self.pools.write_all().await;
        let mut ld_guard = self.liminal_devices.write().await;

        match spawn_blocking!({
            events
                .iter()
                .map(|event| {
                    let uuid = if let Some((name, uuid, pool)) =
                        ld_guard.block_evaluate(&pools_write_all, event)
                    {
                        pools_write_all.insert(name, uuid, pool);
                        Some(uuid)
                    } else {
                        None
                    };
                    match LiminalDevices::block_evaluate_size(&mut pools_write_all, event) {
                        Ok(Some((dev_uuid, diff))) => (uuid, Some((dev_uuid, diff))),
                        Ok(None) => (uuid, None),
                        Err(e) => {
                            warn!("Ignoring device size change handling due to error: {}", e);
                            (uuid, None)
                        }
                    }
                })
                .fold(
                    (Vec::new(), HashMap::new()),
                    |(mut uuids, mut diffs), (uuid, info)| {
                        if let Some(u) = uuid {
                            uuids.push(u);
                        }
                        if let Some((dev_uuid, diff)) = info {
                            diffs.insert(dev_uuid, diff);
                        }
                        (uuids, diffs)
                    },
                )
        }) {
            Ok((uuids, diffs_thread)) => {
                for uuid in uuids {
                    if let Some(guard) = self.pools.read(LockKey::Uuid(uuid)).await {
                        ret_guards.push(guard);
                    }
                }
                diffs.extend(diffs_thread.into_iter());
            }
            Err(e) => {
                warn!("Failed to handle udev events: {}", e);
            }
        };

        (ret_guards, diffs)
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

        let cloned_paths = blockdev_paths
            .iter()
            .map(|p| p.to_path_buf())
            .collect::<Vec<_>>();

        let devices = spawn_blocking!({
            let borrowed_paths = cloned_paths.iter().map(|p| p.as_path()).collect::<Vec<_>>();
            ProcessedPathInfos::try_from(borrowed_paths.as_slice())
        })??;

        let (stratis_devices, unowned_devices) = devices.unpack();

        let maybe_guard = self.pools.read(LockKey::Name(name.clone())).await;
        if let Some(guard) = maybe_guard {
            let (name, uuid, pool) = guard.as_tuple();

            let (this_pool, other_pools) = stratis_devices.partition(uuid);
            other_pools.error_on_not_empty()?;

            create_pool_idempotent_or_err(
                pool,
                &name,
                &this_pool
                    .iter()
                    .map(|(_, info)| info.devnode.as_path())
                    .chain(
                        unowned_devices
                            .unpack()
                            .iter()
                            .map(|info| info.devnode.as_path()),
                    )
                    .collect::<Vec<_>>(),
            )
        } else {
            stratis_devices.error_on_not_empty()?;

            if unowned_devices.is_empty() {
                return Err(StratisError::Msg(
                    "At least one blockdev is required to create a pool.".to_string(),
                ));
            }

            let block_size_summary = unowned_devices.blocksizes();
            if block_size_summary.len() > 1 {
                let err_str = "The devices specified for initializing the pool do not have uniform physcal and logical block sizes.".into();
                return Err(StratisError::Msg(err_str));
            }

            let cloned_name = name.clone();
            let cloned_enc_info = encryption_info.cloned();

            let pool_uuid = {
                let mut pools = self.pools.write_all().await;
                let (pool_uuid, pool) = spawn_blocking!({
                    StratPool::initialize(&cloned_name, unowned_devices, cloned_enc_info.as_ref())
                })??;
                pools.insert(Name::new(name.to_string()), pool_uuid, pool);
                pool_uuid
            };

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
        let pools_read_all = self.pools.read_all().await;
        let mut ld_guard = self.liminal_devices.write().await;
        let unlocked =
            spawn_blocking!(ld_guard.unlock_pool(&pools_read_all, pool_uuid, unlock_method,))??;
        Ok(SetUnlockAction::new(
            unlocked
                .into_iter()
                .map(|(uuid, _)| uuid)
                .collect::<Vec<_>>(),
        ))
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

    async fn stopped_pools(&self) -> HashMap<PoolUuid, StoppedPoolInfo> {
        self.liminal_devices.read().await.stopped_pools()
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

    async fn start_pool(
        &self,
        pool_uuid: PoolUuid,
        unlock_method: Option<UnlockMethod>,
    ) -> StratisResult<StartAction<PoolUuid>> {
        if let Some(lock) = self.pools.read(LockKey::Uuid(pool_uuid)).await {
            let (_, _, pool) = lock.as_tuple();
            if pool.is_encrypted() && unlock_method.is_none() {
                return Err(StratisError::Msg(format!(
                    "Pool with UUID {} is encrypted but no unlock method was provided",
                    pool_uuid,
                )));
            } else if !pool.is_encrypted() && unlock_method.is_some() {
                return Err(StratisError::Msg(format!(
                    "Pool with UUID {} is not encrypted but an unlock method was provided",
                    pool_uuid,
                )));
            } else {
                Ok(StartAction::Identity)
            }
        } else {
            let mut pools = self.pools.write_all().await;
            let (name, pool) =
                self.liminal_devices
                    .write()
                    .await
                    .start_pool(&pools, pool_uuid, unlock_method)?;
            pools.insert(name, pool_uuid, pool);
            Ok(StartAction::Started(pool_uuid))
        }
    }

    async fn stop_pool(&self, pool_uuid: PoolUuid) -> StratisResult<StopAction<PoolUuid>> {
        let mut pools = self.pools.write_all().await;
        if let Some((name, mut pool)) = pools.remove_by_uuid(pool_uuid) {
            if let Err(e) = self
                .liminal_devices
                .write()
                .await
                .stop_pool(&name, pool_uuid, &mut pool)
            {
                pools.insert(name, pool_uuid, pool);
                return Err(e);
            } else {
                return Ok(StopAction::Stopped(pool_uuid));
            }
        }

        drop(pools);

        if self
            .liminal_devices
            .read()
            .await
            .stopped_pools()
            .get(&pool_uuid)
            .is_some()
        {
            Ok(StopAction::Identity)
        } else {
            Err(StratisError::Msg(format!(
                "Pool with UUID {} could not be found and cannot be stopped",
                pool_uuid,
            )))
        }
    }

    async fn refresh_state(&self) -> StratisResult<()> {
        let mut pools = self.pools.write_all().await;
        *pools = Table::default();
        let mut lim = self.liminal_devices.write().await;
        *lim = LiminalDevices::default();
        let pools_set_up = lim.setup_pools(find_all()?);
        for (name, uuid, pool) in pools_set_up {
            pools.insert(name, uuid, pool);
        }
        Ok(())
    }

    fn is_sim(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod test {
    use std::{env, error::Error, path::Path};

    use devicemapper::Sectors;

    use crate::engine::{
        engine::Pool,
        strat_engine::{
            backstore::crypt_metadata_size,
            cmd,
            ns::unshare_namespace,
            tests::{crypt, loopbacked, real, FailDevice},
        },
        types::{ActionAvailability, EngineAction, KeyDescription},
    };

    use super::*;

    /// Verify that a pool rename causes the pool metadata to get the new name.
    fn test_pool_rename(paths: &[&Path]) {
        unshare_namespace().unwrap();
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
            engine: &StratEngine,
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

                fail_device.start_failing(*crypt_metadata_size().sectors())?;
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

            Ok(())
        }

        unshare_namespace()?;
        let engine = StratEngine::initialize()?;
        let uuid =
            test_async!(engine.create_pool(name, paths_with_fail_device, Some(encryption_info)))?
                .changed()
                .ok_or_else(|| {
                    Box::new(StratisError::Msg(
                        "Pool should be newly created".to_string(),
                    ))
                })?;

        let res = needs_clean_up(&engine, uuid, fail_device, operation);

        test_async!(engine.stop_pool(uuid))?;
        res?;

        test_async!(engine.start_pool(uuid, Some(unlock_method)))?;
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

    /// Test creating a pool and stopping it. Check that the pool is stopped and
    /// then restart the engine, check that it is still stopped, and then start it.
    fn test_start_stop(paths: &[&Path]) {
        let engine = StratEngine::initialize().unwrap();
        let name = "pool_name";
        let uuid = test_async!(engine.create_pool(name, paths, None))
            .unwrap()
            .changed()
            .unwrap();
        assert!(test_async!(engine.stop_pool(uuid)).unwrap().is_changed());
        assert_eq!(test_async!(engine.stopped_pools()).len(), 1);
        assert_eq!(test_async!(engine.pools()).len(), 0);

        engine.teardown().unwrap();

        let engine = StratEngine::initialize().unwrap();
        assert_eq!(test_async!(engine.stopped_pools()).len(), 1);
        assert_eq!(test_async!(engine.pools()).len(), 0);

        assert!(test_async!(engine.start_pool(uuid, None))
            .unwrap()
            .is_changed());
        assert_eq!(test_async!(engine.stopped_pools()).len(), 0);
        assert_eq!(test_async!(engine.pools()).len(), 1);
    }

    #[test]
    fn loop_test_start_stop() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_start_stop,
        );
    }

    #[test]
    fn real_test_start_stop() {
        real::test_with_spec(&real::DeviceLimits::AtLeast(2, None, None), test_start_stop);
    }
}

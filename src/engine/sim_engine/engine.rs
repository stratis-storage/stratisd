// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{hash_map::RandomState, HashMap, HashSet},
    iter::FromIterator,
    path::Path,
    sync::Arc,
};

use async_trait::async_trait;
use futures::executor::block_on;
use serde_json::{json, Value};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

use crate::{
    engine::{
        engine::{Engine, Report},
        shared::{create_pool_idempotent_or_err, validate_name, validate_paths},
        sim_engine::{keys::SimKeyActions, pool::SimPool},
        structures::{
            AllLockReadGuard, AllLockWriteGuard, AllOrSomeLock, ExclusiveGuard, Lockable,
            SharedGuard, SomeLockReadGuard, SomeLockWriteGuard,
        },
        types::{
            CreateAction, DeleteAction, DevUuid, EncryptionInfo, FilesystemUuid, LockKey,
            LockedPoolInfo, Name, PoolUuid, RenameAction, ReportType, SetUnlockAction,
            StratFilesystemDiff, ThinPoolDiff, UdevEngineEvent, UnlockMethod,
        },
    },
    stratis::{StratisError, StratisResult},
};

#[derive(Debug)]
pub struct SimEngine {
    pools: AllOrSomeLock<PoolUuid, SimPool>,
    key_handler: Lockable<Arc<RwLock<SimKeyActions>>>,
}

impl Default for SimEngine {
    fn default() -> Self {
        SimEngine {
            pools: AllOrSomeLock::default(),
            key_handler: Lockable::new_shared(SimKeyActions::default()),
        }
    }
}

impl<'a> Into<Value> for &'a SimEngine {
    // Precondition: SimPool Into<Value> impl return value always pattern matches
    // Value::Object(_)
    fn into(self) -> Value {
        json!({
            "pools": Value::Array(
                block_on(self.pools.read_all()).iter().map(|(name, uuid, pool)| {
                    let json = json!({
                        "pool_uuid": uuid.to_string(),
                        "name": name.to_string(),
                    });
                    let pool_json = pool.into();
                    if let (Value::Object(mut map), Value::Object(submap)) = (json, pool_json) {
                        map.extend(submap.into_iter());
                        Value::Object(map)
                    } else {
                        unreachable!("json!() output is always JSON object");
                    }
                })
                .collect()
            ),
            "errored_pools": json!([]),
            "hopeless_devices": json!([]),
        })
    }
}

impl Report for SimEngine {
    fn engine_state_report(&self) -> Value {
        self.into()
    }

    fn get_report(&self, report_type: ReportType) -> Value {
        match report_type {
            ReportType::ErroredPoolDevices => json!({
                "errored_pools": json!([]),
                "hopeless_devices": json!([]),
            }),
        }
    }
}

#[async_trait]
impl Engine for SimEngine {
    type Pool = SimPool;
    type KeyActions = SimKeyActions;

    async fn create_pool(
        &self,
        name: &str,
        blockdev_paths: &[&Path],
        redundancy: Option<u16>,
        encryption_info: Option<&EncryptionInfo>,
    ) -> StratisResult<CreateAction<PoolUuid>> {
        let redundancy = calculate_redundancy!(redundancy);

        validate_name(name)?;
        let name = Name::new(name.to_owned());

        validate_paths(blockdev_paths)?;

        if let Some(key_desc) = encryption_info.and_then(|ei| ei.key_description()) {
            if !self.key_handler.read().await.contains_key(key_desc) {
                return Err(StratisError::Msg(format!(
                    "Key {} was not found in the keyring",
                    key_desc.as_application_str()
                )));
            }
        }

        let guard = self.pools.read(LockKey::Name(name.clone())).await;
        match guard.as_ref().map(|g| g.as_tuple()) {
            Some((_, _, pool)) => create_pool_idempotent_or_err(pool, &name, blockdev_paths),
            None => {
                if blockdev_paths.is_empty() {
                    Err(StratisError::Msg(
                        "At least one blockdev is required to create a pool.".to_string(),
                    ))
                } else {
                    let device_set: HashSet<_, RandomState> = HashSet::from_iter(blockdev_paths);
                    let devices = device_set.into_iter().cloned().collect::<Vec<_>>();

                    let (pool_uuid, pool) = SimPool::new(&devices, redundancy, encryption_info);

                    self.pools.write_all().await.insert(
                        Name::new(name.to_owned()),
                        pool_uuid,
                        pool,
                    );

                    Ok(CreateAction::Created(pool_uuid))
                }
            }
        }
    }

    async fn handle_events(
        &self,
        _: Vec<UdevEngineEvent>,
    ) -> Vec<SomeLockReadGuard<PoolUuid, Self::Pool>> {
        Vec::new()
    }

    async fn destroy_pool(&self, uuid: PoolUuid) -> StratisResult<DeleteAction<PoolUuid>> {
        if let Some(pool) = self.pools.read(LockKey::Uuid(uuid)).await {
            if pool.has_filesystems() {
                return Err(StratisError::Msg("filesystems remaining on pool".into()));
            }
            drop(pool);
            self.pools
                .write_all()
                .await
                .remove_by_uuid(uuid)
                .expect("Must succeed since self.pool.get_by_uuid() returned a value")
                .1
                .destroy()?;
            Ok(DeleteAction::Deleted(uuid))
        } else {
            Ok(DeleteAction::Identity)
        }
    }

    async fn rename_pool(
        &self,
        uuid: PoolUuid,
        new_name: &str,
    ) -> StratisResult<RenameAction<PoolUuid>> {
        let new_name = Name::new(new_name.to_owned());
        rename_pool_pre_idem!(self; uuid; new_name.clone());

        let mut guard = self.pools.write_all().await;

        let (_, pool) = guard
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pools.get_by_uuid() returned a value");

        guard.insert(new_name, uuid, pool);
        Ok(RenameAction::Renamed(uuid))
    }

    async fn unlock_pool(
        &self,
        _pool_uuid: PoolUuid,
        _unlock_method: UnlockMethod,
    ) -> StratisResult<SetUnlockAction<DevUuid>> {
        Ok(SetUnlockAction::empty())
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
        HashMap::new()
    }

    async fn pools(&self) -> AllLockReadGuard<PoolUuid, Self::Pool> {
        self.pools.read_all().await
    }

    async fn pools_mut(&self) -> AllLockWriteGuard<PoolUuid, Self::Pool> {
        self.pools.write_all().await
    }

    async fn get_events(&self) -> StratisResult<HashSet<PoolUuid>> {
        Ok(HashSet::new())
    }

    async fn pool_evented(&self, _: Option<&HashSet<PoolUuid>>) -> HashMap<PoolUuid, ThinPoolDiff> {
        HashMap::default()
    }

    async fn fs_evented(
        &self,
        _: Option<&HashSet<PoolUuid>>,
    ) -> HashMap<FilesystemUuid, StratFilesystemDiff> {
        HashMap::default()
    }

    async fn get_key_handler(&self) -> SharedGuard<OwnedRwLockReadGuard<Self::KeyActions>> {
        self.key_handler.read().await
    }

    async fn get_key_handler_mut(&self) -> ExclusiveGuard<OwnedRwLockWriteGuard<Self::KeyActions>> {
        self.key_handler.write().await
    }

    fn is_sim(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::{
        engine::{Engine, Pool},
        types::{EngineAction, RenameAction},
    };

    use super::*;

    #[test]
    /// When an engine has no pools, any name lookup should fail
    fn get_pool_err() {
        assert!(
            test_async!(SimEngine::default().get_pool(LockKey::Uuid(PoolUuid::new_v4()))).is_none()
        );
    }

    #[test]
    /// When an engine has no pools, destroying any pool must succeed
    fn destroy_pool_empty() {
        assert!(test_async!(SimEngine::default().destroy_pool(PoolUuid::new_v4())).is_ok());
    }

    #[test]
    /// Destroying an empty pool should succeed.
    fn destroy_empty_pool() {
        let engine = SimEngine::default();
        let uuid = test_async!(engine.create_pool(
            "name",
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            None,
        ))
        .unwrap()
        .changed()
        .unwrap();
        assert!(test_async!(engine.destroy_pool(uuid)).is_ok());
    }

    #[test]
    /// Destroying a pool with devices should succeed
    fn destroy_pool_w_devices() {
        let engine = SimEngine::default();
        let uuid = test_async!(engine.create_pool("name", strs_to_paths!(["/s/d"]), None, None))
            .unwrap()
            .changed()
            .unwrap();
        assert!(test_async!(engine.destroy_pool(uuid)).is_ok());
    }

    #[test]
    /// Destroying a pool with filesystems should fail
    fn destroy_pool_w_filesystem() {
        let engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = test_async!(engine.create_pool(pool_name, strs_to_paths!(["/s/d"]), None, None))
            .unwrap()
            .changed()
            .unwrap();
        {
            let mut pool = test_async!(engine.get_mut_pool(LockKey::Uuid(uuid))).unwrap();
            pool.create_filesystems(pool_name, uuid, &[("test", None)])
                .unwrap();
        }
        assert!(test_async!(engine.destroy_pool(uuid)).is_err());
    }

    #[test]
    /// Creating a new pool with the same name and arguments should return
    /// identity.
    fn create_pool_name_collision() {
        let name = "name";
        let engine = SimEngine::default();
        let devices = strs_to_paths!(["/s/d"]);
        test_async!(engine.create_pool(name, devices, None, None)).unwrap();
        assert_matches!(
            test_async!(engine.create_pool(name, devices, None, None)),
            Ok(CreateAction::Identity)
        );
    }

    #[test]
    /// Creating a new pool with the same name and different arguments should fail
    fn create_pool_name_collision_different_args() {
        let name = "name";
        let engine = SimEngine::default();
        test_async!(engine.create_pool(name, strs_to_paths!(["/s/d"]), None, None)).unwrap();
        assert!(test_async!(engine.create_pool(
            name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            None,
        ))
        .is_err());
    }

    #[test]
    /// Creating a pool with duplicate devices should succeed
    fn create_pool_duplicate_devices() {
        let path = "/s/d";
        let engine = SimEngine::default();
        assert_matches!(
            test_async!(engine.create_pool("name", strs_to_paths!([path, path]), None, None))
                .unwrap()
                .changed()
                .map(|uuid| test_async!(engine.get_pool(LockKey::Uuid(uuid)))
                    .unwrap()
                    .blockdevs()
                    .len()),
            Some(1)
        );
    }

    #[test]
    /// Creating a pool with an impossible raid level should fail
    fn create_pool_max_u16_raid() {
        let engine = SimEngine::default();
        assert!(test_async!(engine.create_pool(
            "name",
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            Some(std::u16::MAX),
            None,
        ))
        .is_err());
    }

    #[test]
    /// Renaming a pool on an empty engine always works
    fn rename_empty() {
        let engine = SimEngine::default();
        assert_matches!(
            test_async!(engine.rename_pool(PoolUuid::new_v4(), "new_name")),
            Ok(RenameAction::NoSource)
        );
    }

    #[test]
    /// Renaming a pool to itself always works
    fn rename_identity() {
        let name = "name";
        let engine = SimEngine::default();
        let uuid = test_async!(engine.create_pool(
            name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            None,
        ))
        .unwrap()
        .changed()
        .unwrap();
        assert_eq!(
            test_async!(engine.rename_pool(uuid, name)).unwrap(),
            RenameAction::Identity
        );
    }

    #[test]
    /// Renaming a pool to another pool should work if new name not taken
    fn rename_happens() {
        let engine = SimEngine::default();
        let uuid = test_async!(engine.create_pool(
            "old_name",
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            None,
        ))
        .unwrap()
        .changed()
        .unwrap();
        assert_eq!(
            test_async!(engine.rename_pool(uuid, "new_name")).unwrap(),
            RenameAction::Renamed(uuid)
        );
    }

    #[test]
    /// Renaming a pool to another pool should fail if new name taken
    fn rename_fails() {
        let new_name = "new_name";
        let engine = SimEngine::default();
        let uuid = test_async!(engine.create_pool(
            "old_name",
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            None,
        ))
        .unwrap()
        .changed()
        .unwrap();
        test_async!(engine.create_pool(
            new_name,
            strs_to_paths!(["/dev/four", "/dev/five", "/dev/six"]),
            None,
            None,
        ))
        .unwrap();
        assert!(test_async!(engine.rename_pool(uuid, new_name)).is_err());
    }

    #[test]
    /// Renaming should succeed if old_name absent, new present
    fn rename_no_op() {
        let new_name = "new_name";
        let engine = SimEngine::default();
        test_async!(engine.create_pool(
            new_name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            None,
        ))
        .unwrap();
        assert_matches!(
            test_async!(engine.rename_pool(PoolUuid::new_v4(), new_name)),
            Ok(RenameAction::NoSource)
        );
    }
}

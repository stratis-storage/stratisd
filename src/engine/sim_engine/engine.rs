// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{hash_map::RandomState, HashMap, HashSet},
    os::fd::RawFd,
    path::Path,
    sync::Arc,
};

use async_trait::async_trait;
use futures::executor::block_on;
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::{
    engine::{
        engine::{Engine, HandleEvents, KeyActions, Pool, Report},
        shared::{create_pool_idempotent_or_err, validate_name, validate_paths},
        sim_engine::{keys::SimKeyActions, pool::SimPool},
        structures::{
            AllLockReadGuard, AllLockWriteGuard, AllOrSomeLock, Lockable, SomeLockReadGuard,
            SomeLockWriteGuard, Table,
        },
        types::{
            CreateAction, DeleteAction, DevUuid, Features, FilesystemUuid, InputEncryptionInfo,
            IntegritySpec, LockedPoolsInfo, Name, PoolDevice, PoolDiff, PoolIdentifier, PoolUuid,
            RenameAction, ReportType, SetUnlockAction, StartAction, StopAction, StoppedPoolInfo,
            StoppedPoolsInfo, StratFilesystemDiff, TokenUnlockMethod, UdevEngineEvent,
            UnlockMethod, ValidatedIntegritySpec,
        },
        StratSigblockVersion,
    },
    stratis::{StratisError, StratisResult},
};

use super::shared::convert_encryption_info;

#[derive(Debug)]
pub struct SimEngine {
    pools: AllOrSomeLock<PoolUuid, SimPool>,
    key_handler: Arc<SimKeyActions>,
    stopped_pools: Lockable<Arc<RwLock<Table<PoolUuid, SimPool>>>>,
}

impl Default for SimEngine {
    fn default() -> Self {
        SimEngine {
            pools: AllOrSomeLock::default(),
            key_handler: Arc::new(SimKeyActions::default()),
            stopped_pools: Lockable::new_shared(Table::default()),
        }
    }
}

impl Into<Value> for &SimEngine {
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
            "stopped_pools": Value::Array(
                (*block_on(self.stopped_pools.read())).iter().map(|(name, uuid, pool)| {
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
                .collect(),
            ),
            "partially_constructed_pools": Value::Array(Vec::new())
        })
    }
}

impl Report for SimEngine {
    fn engine_state_report(&self) -> Value {
        self.into()
    }

    fn get_report(&self, report_type: ReportType) -> Value {
        match report_type {
            ReportType::StoppedPools => {
                json!({
                    "stopped_pools": (*block_on(self.stopped_pools.read())).iter().map(|(name, uuid, pool)| {
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
                    .collect::<Vec<_>>()
                })
            }
        }
    }
}

#[async_trait]
impl Engine for SimEngine {
    async fn create_pool(
        &self,
        name: &str,
        blockdev_paths: &[&Path],
        encryption_info: Option<&InputEncryptionInfo>,
        integrity_spec: IntegritySpec,
    ) -> StratisResult<CreateAction<PoolUuid>> {
        validate_name(name)?;
        let name = Name::new(name.to_owned());

        validate_paths(blockdev_paths)?;

        let integrity_spec = ValidatedIntegritySpec::try_from(integrity_spec)?;

        let converted_ei = convert_encryption_info(encryption_info, Some(&self.key_handler))?;

        let guard = self.pools.read(PoolIdentifier::Name(name.clone())).await;
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

                    let (pool_uuid, pool) =
                        SimPool::new(&devices, converted_ei.as_ref(), integrity_spec);

                    self.pools.modify_all().await.insert(
                        Name::new(name.to_owned()),
                        pool_uuid,
                        pool,
                    );

                    Ok(CreateAction::Created(pool_uuid))
                }
            }
        }
    }

    async fn handle_events(&self, _: Vec<UdevEngineEvent>) -> HandleEvents<dyn Pool> {
        (Vec::new(), HashMap::new())
    }

    async fn destroy_pool(&self, uuid: PoolUuid) -> StratisResult<DeleteAction<PoolUuid>> {
        if let Some(pool) = self.pools.read(PoolIdentifier::Uuid(uuid)).await {
            if pool.has_filesystems() {
                return Err(StratisError::Msg("filesystems remaining on pool".into()));
            }
            drop(pool);
            self.pools
                .modify_all()
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

        let mut guard = self.pools.modify_all().await;

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
        key: PoolIdentifier<PoolUuid>,
    ) -> Option<SomeLockReadGuard<PoolUuid, dyn Pool>> {
        get_pool!(self; key).map(|l| l.into_dyn())
    }

    async fn get_mut_pool(
        &self,
        key: PoolIdentifier<PoolUuid>,
    ) -> Option<SomeLockWriteGuard<PoolUuid, dyn Pool>> {
        get_mut_pool!(self; key).map(|l| l.into_dyn())
    }

    async fn locked_pools(&self) -> LockedPoolsInfo {
        LockedPoolsInfo::default()
    }

    async fn stopped_pools(&self) -> StoppedPoolsInfo {
        self.stopped_pools.read().await.iter().fold(
            StoppedPoolsInfo::default(),
            |mut st, (name, uuid, pool)| {
                st.name_to_uuid.insert(name.clone(), *uuid);
                st.uuid_to_name.insert(*uuid, name.clone());
                st.stopped.insert(
                    *uuid,
                    StoppedPoolInfo {
                        info: pool.encryption_info_legacy(),
                        devices: pool
                            .blockdevs()
                            .into_iter()
                            .map(|(dev_uuid, _, bd)| PoolDevice {
                                devnode: bd.devnode().to_path_buf(),
                                uuid: dev_uuid,
                            })
                            .collect::<Vec<_>>(),
                        metadata_version: Some(StratSigblockVersion::V2),
                        features: Some(Features {
                            encryption: pool.is_encrypted(),
                        }),
                    },
                );
                st
            },
        )
    }

    async fn pools(&self) -> AllLockReadGuard<PoolUuid, dyn Pool> {
        self.pools.read_all().await.into_dyn()
    }

    async fn pools_mut(&self) -> AllLockWriteGuard<PoolUuid, dyn Pool> {
        self.pools.write_all().await.into_dyn()
    }

    async fn get_events(&self) -> StratisResult<HashSet<PoolUuid>> {
        Ok(HashSet::new())
    }

    async fn pool_evented(&self, _: Option<&HashSet<PoolUuid>>) -> HashMap<PoolUuid, PoolDiff> {
        HashMap::default()
    }

    async fn fs_evented(
        &self,
        _: Option<&HashSet<PoolUuid>>,
    ) -> HashMap<FilesystemUuid, StratFilesystemDiff> {
        HashMap::default()
    }

    async fn get_key_handler(&self) -> Arc<dyn KeyActions> {
        Arc::clone(&self.key_handler) as Arc<dyn KeyActions>
    }

    async fn start_pool(
        &self,
        id: PoolIdentifier<PoolUuid>,
        token_slot: TokenUnlockMethod,
        passphrase_fd: Option<RawFd>,
    ) -> StratisResult<StartAction<PoolUuid>> {
        if let Some(guard) = self.pools.read(id.clone()).await {
            let (_, pool_uuid, pool) = guard.as_tuple();
            if !pool.is_encrypted() && token_slot.is_some() {
                return Err(StratisError::Msg(format!(
                    "Pool with UUID {pool_uuid} is not encrypted but an unlock method was provided"
                )));
            } else if !pool.is_encrypted() && passphrase_fd.is_some() {
                return Err(StratisError::Msg(format!(
                    "Pool with UUID {pool_uuid} is not encrypted but a passphrase was provided"
                )));
            } else {
                Ok(StartAction::Identity)
            }
        } else {
            let (name, pool_uuid, pool) = match id {
                PoolIdentifier::Name(n) => self
                    .stopped_pools
                    .write()
                    .await
                    .remove_by_name(&n)
                    .ok_or_else(|| {
                        StratisError::Msg(format!(
                            "Pool with name {n} was not found and cannot be started"
                        ))
                    })
                    .map(|(u, p)| (n, u, p))?,
                PoolIdentifier::Uuid(u) => self
                    .stopped_pools
                    .write()
                    .await
                    .remove_by_uuid(u)
                    .ok_or_else(|| {
                        StratisError::Msg(format!(
                            "Pool with UUID {u} was not found and cannot be started"
                        ))
                    })
                    .map(|(n, p)| (n, u, p))?,
            };
            if pool.is_encrypted() && token_slot.is_none() {
                self.stopped_pools
                    .write()
                    .await
                    .insert(name, pool_uuid, pool);
                return Err(StratisError::Msg(format!(
                    "Pool with UUID {pool_uuid} is encrypted but no unlock method was provided"
                )));
            } else if !pool.is_encrypted() && token_slot.is_some() {
                self.stopped_pools
                    .write()
                    .await
                    .insert(name, pool_uuid, pool);
                return Err(StratisError::Msg(format!(
                    "Pool with UUID {pool_uuid} is not encrypted but an unlock method was provided"
                )));
            } else if !pool.is_encrypted() && passphrase_fd.is_some() {
                self.stopped_pools
                    .write()
                    .await
                    .insert(name, pool_uuid, pool);
                return Err(StratisError::Msg(format!(
                    "Pool with UUID {pool_uuid} is not encrypted but a passphrase was provided"
                )));
            }
            self.pools.modify_all().await.insert(name, pool_uuid, pool);
            Ok(StartAction::Started(pool_uuid))
        }
    }

    async fn stop_pool(
        &self,
        pool_id: PoolIdentifier<PoolUuid>,
        _: bool,
    ) -> StratisResult<StopAction<PoolUuid>> {
        let is_stopped = match pool_id {
            PoolIdentifier::Name(ref n) => self.stopped_pools.read().await.get_by_name(n).is_some(),
            PoolIdentifier::Uuid(u) => self.stopped_pools.read().await.get_by_uuid(u).is_some(),
        };
        if is_stopped {
            return Ok(StopAction::Identity);
        }

        let pool_entry = match pool_id {
            PoolIdentifier::Name(ref n) => self
                .pools
                .modify_all()
                .await
                .remove_by_name(n)
                .map(|(u, p)| (n.clone(), u, p)),
            PoolIdentifier::Uuid(u) => self
                .pools
                .modify_all()
                .await
                .remove_by_uuid(u)
                .map(|(n, p)| (n, u, p)),
        };
        if let Some((name, pool_uuid, pool)) = pool_entry {
            self.stopped_pools
                .write()
                .await
                .insert(name, pool_uuid, pool);
            Ok(StopAction::Stopped(pool_uuid))
        } else {
            Err(StratisError::Msg(format!(
                "Pool with ID {pool_id} was not found and cannot be stopped"
            )))
        }
    }

    async fn refresh_state(&self) -> StratisResult<()> {
        Ok(())
    }

    fn is_sim(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::{
        engine::Engine,
        types::{EngineAction, RenameAction},
    };

    use super::*;

    #[test]
    /// When an engine has no pools, any name lookup should fail
    fn get_pool_err() {
        assert!(test_async!(
            SimEngine::default().get_pool(PoolIdentifier::Uuid(PoolUuid::new_v4()))
        )
        .is_none());
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
            IntegritySpec::default(),
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
        let uuid = test_async!(engine.create_pool(
            "name",
            strs_to_paths!(["/s/d"]),
            None,
            IntegritySpec::default()
        ))
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
        let uuid = test_async!(engine.create_pool(
            pool_name,
            strs_to_paths!(["/s/d"]),
            None,
            IntegritySpec::default()
        ))
        .unwrap()
        .changed()
        .unwrap();
        {
            let mut pool = test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(uuid))).unwrap();
            pool.create_filesystems(pool_name, uuid, &[("test", None, None)])
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
        test_async!(engine.create_pool(name, devices, None, IntegritySpec::default())).unwrap();
        assert_matches!(
            test_async!(engine.create_pool(name, devices, None, IntegritySpec::default())),
            Ok(CreateAction::Identity)
        );
    }

    #[test]
    /// Creating a new pool with the same name and different arguments should fail
    fn create_pool_name_collision_different_args() {
        let name = "name";
        let engine = SimEngine::default();
        test_async!(engine.create_pool(
            name,
            strs_to_paths!(["/s/d"]),
            None,
            IntegritySpec::default()
        ))
        .unwrap();
        assert!(test_async!(engine.create_pool(
            name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            IntegritySpec::default(),
        ))
        .is_err());
    }

    #[test]
    /// Creating a pool with duplicate devices should succeed
    fn create_pool_duplicate_devices() {
        let path = "/s/d";
        let engine = SimEngine::default();
        assert_matches!(
            test_async!(engine.create_pool(
                "name",
                strs_to_paths!([path, path]),
                None,
                IntegritySpec::default()
            ))
            .unwrap()
            .changed()
            .map(
                |uuid| test_async!(engine.get_pool(PoolIdentifier::Uuid(uuid)))
                    .unwrap()
                    .blockdevs()
                    .len()
            ),
            Some(1)
        );
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
            IntegritySpec::default(),
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
            IntegritySpec::default(),
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
            IntegritySpec::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        test_async!(engine.create_pool(
            new_name,
            strs_to_paths!(["/dev/four", "/dev/five", "/dev/six"]),
            None,
            IntegritySpec::default(),
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
            IntegritySpec::default(),
        ))
        .unwrap();
        assert_matches!(
            test_async!(engine.rename_pool(PoolUuid::new_v4(), new_name)),
            Ok(RenameAction::NoSource)
        );
    }
}

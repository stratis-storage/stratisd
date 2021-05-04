// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{hash_map::RandomState, HashMap, HashSet},
    iter::FromIterator,
    path::Path,
};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{
    engine::{
        engine::{Engine, KeyActions, Pool, Report},
        shared::{create_pool_idempotent_or_err, validate_name, validate_paths},
        sim_engine::{keys::SimKeyActions, pool::SimPool},
        structures::{Lockable, Table},
        types::{
            CreateAction, DeleteAction, DevUuid, EncryptionInfo, LockedPoolInfo, Name, PoolUuid,
            RenameAction, ReportType, SetUnlockAction, UdevEngineEvent, UnlockMethod,
        },
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

#[derive(Debug, Default)]
pub struct SimEngine {
    pools: Lockable<Table<PoolUuid, Lockable<SimPool>>>,
    key_handler: Lockable<SimKeyActions>,
}

impl<T> Default for Lockable<T>
where
    T: Default + Send + Sync,
{
    fn default() -> Self {
        Lockable::new(T::default())
    }
}

#[async_trait]
impl Report for SimEngine {
    // Precondition: SimPool Into<Value> impl return value always pattern matches
    // Value::Object(_)
    async fn engine_state_report(&self) -> Value {
        json!({
            "pools": Value::Array({
                let mut pools = Vec::new();
                for (name, uuid, pool) in self.pools.read().await.iter() {
                    let json = json!({
                        "pool_uuid": uuid.to_string(),
                        "name": name.to_string(),
                    });
                    let pool_json = (&*pool.read().await).into();
                    if let (Value::Object(mut map), Value::Object(submap)) = (json, pool_json) {
                        map.extend(submap.into_iter());
                        pools.push(Value::Object(map));
                    } else {
                        unreachable!("json!() output is always JSON object");
                    }
                }
                pools
            }),
            "errored_pools": json!([]),
            "hopeless_devices": json!([]),
        })
    }

    async fn get_report(&self, report_type: ReportType) -> Value {
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

        if let Some(ref key_desc) = encryption_info.key_description {
            if !self.key_handler.read().await.contains_key(key_desc) {
                return Err(StratisError::Engine(
                    ErrorEnum::NotFound,
                    format!(
                        "Key {} was not found in the keyring",
                        key_desc.as_application_str()
                    ),
                ));
            }
        }

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
            let device_set: HashSet<_, RandomState> = HashSet::from_iter(blockdev_paths);
            let devices = device_set.into_iter().cloned().collect::<Vec<&Path>>();

            let (pool_uuid, pool) = SimPool::new(&devices, redundancy, encryption_info);

            self.pools.write().await.insert(
                Name::new(name.to_owned()),
                pool_uuid,
                Lockable::new(pool),
            );

            Ok(CreateAction::Created(pool_uuid))
        }
    }

    async fn handle_event(
        &self,
        _event: &UdevEngineEvent,
    ) -> Option<(Name, PoolUuid, Lockable<dyn Pool>)> {
        None
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
        self.pools
            .write()
            .await
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pool.get_by_uuid() returned a value")
            .1
            .write()
            .await
            .destroy()?;
        Ok(DeleteAction::Deleted(uuid))
    }

    async fn rename_pool(
        &self,
        uuid: PoolUuid,
        new_name: &str,
    ) -> StratisResult<RenameAction<PoolUuid>> {
        rename_pool_pre_idem!(self; uuid; new_name);

        let mut pool_lock = self.pools.write().await;
        let (_, pool) = pool_lock
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pools.get_by_uuid() returned a value");

        pool_lock.insert(Name::new(new_name.to_owned()), uuid, pool);
        Ok(RenameAction::Renamed(uuid))
    }

    async fn unlock_pool(
        &self,
        _pool_uuid: PoolUuid,
        _unlock_method: UnlockMethod,
    ) -> StratisResult<SetUnlockAction<DevUuid>> {
        Ok(SetUnlockAction::empty())
    }

    async fn get_pool(&self, uuid: PoolUuid) -> Option<(Name, Lockable<dyn Pool>)> {
        get_pool!(self; uuid)
    }

    async fn locked_pools(&self) -> HashMap<PoolUuid, LockedPoolInfo> {
        HashMap::new()
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
        Ok(())
    }

    fn get_key_handler(&self) -> Lockable<dyn KeyActions> {
        self.key_handler.clone().into_dyn_key_handler()
    }

    fn is_sim(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {

    use std::{self, path::Path};

    use futures::executor::block_on;

    use crate::{
        engine::{
            types::{EngineAction, RenameAction},
            Engine,
        },
        stratis::{ErrorEnum, StratisError},
    };

    use super::*;

    #[test]
    /// When an engine has no pools, any name lookup should fail
    fn get_pool_err() {
        assert_matches!(
            block_on(SimEngine::default().get_pool(PoolUuid::new_v4())),
            None
        );
    }

    #[test]
    /// When an engine has no pools, destroying any pool must succeed
    fn destroy_pool_empty() {
        assert_matches!(
            block_on(SimEngine::default().destroy_pool(PoolUuid::new_v4())),
            Ok(_)
        );
    }

    #[test]
    /// Destroying an empty pool should succeed.
    fn destroy_empty_pool() {
        let engine = SimEngine::default();
        let uuid = block_on(engine.create_pool(
            "name",
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            &EncryptionInfo::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        assert_matches!(block_on(engine.destroy_pool(uuid)), Ok(_));
    }

    #[test]
    /// Destroying a pool with devices should succeed
    fn destroy_pool_w_devices() {
        let engine = SimEngine::default();
        let uuid = block_on(engine.create_pool(
            "name",
            strs_to_paths!(["/s/d"]),
            None,
            &EncryptionInfo::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        assert_matches!(block_on(engine.destroy_pool(uuid)), Ok(_));
    }

    #[test]
    /// Destroying a pool with filesystems should fail
    fn destroy_pool_w_filesystem() {
        let engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = block_on(engine.create_pool(
            pool_name,
            strs_to_paths!(["/s/d"]),
            None,
            &EncryptionInfo::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        {
            let pool = block_on(engine.get_pool(uuid)).unwrap().1;
            lock!(pool, write)
                .create_filesystems(pool_name, uuid, &[("test", None)])
                .unwrap();
        }
        assert_matches!(block_on(engine.destroy_pool(uuid)), Err(_));
    }

    #[test]
    /// Creating a new pool with the same name and arguments should return
    /// identity.
    fn create_pool_name_collision() {
        let name = "name";
        let engine = SimEngine::default();
        let devices = strs_to_paths!(["/s/d"]);
        block_on(engine.create_pool(name, devices, None, &EncryptionInfo::default())).unwrap();
        assert_matches!(
            block_on(engine.create_pool(name, devices, None, &EncryptionInfo::default())),
            Ok(CreateAction::Identity)
        );
    }

    #[test]
    /// Creating a new pool with the same name and different arguments should fail
    fn create_pool_name_collision_different_args() {
        let name = "name";
        let engine = SimEngine::default();
        block_on(engine.create_pool(
            name,
            strs_to_paths!(["/s/d"]),
            None,
            &EncryptionInfo::default(),
        ))
        .unwrap();
        assert_matches!(
            block_on(engine.create_pool(
                name,
                strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
                None,
                &EncryptionInfo::default(),
            )),
            Err(StratisError::Engine(ErrorEnum::Invalid, _))
        );
    }

    #[test]
    /// Creating a pool with duplicate devices should succeed
    fn create_pool_duplicate_devices() {
        let path = "/s/d";
        let engine = SimEngine::default();
        assert_matches!(
            block_on(engine.create_pool(
                "name",
                strs_to_paths!([path, path]),
                None,
                &EncryptionInfo::default()
            ))
            .unwrap()
            .changed()
            .map(
                |uuid| lock!(block_on(engine.get_pool(uuid)).unwrap().1, read)
                    .blockdevs()
                    .len()
            ),
            Some(1)
        );
    }

    #[test]
    /// Creating a pool with an impossible raid level should fail
    fn create_pool_max_u16_raid() {
        let engine = SimEngine::default();
        assert_matches!(
            block_on(engine.create_pool(
                "name",
                strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
                Some(std::u16::MAX),
                &EncryptionInfo::default(),
            )),
            Err(_)
        );
    }

    #[test]
    /// Renaming a pool on an empty engine always works
    fn rename_empty() {
        let engine = SimEngine::default();
        assert_matches!(
            block_on(engine.rename_pool(PoolUuid::new_v4(), "new_name")),
            Ok(RenameAction::NoSource)
        );
    }

    #[test]
    /// Renaming a pool to itself always works
    fn rename_identity() {
        let name = "name";
        let engine = SimEngine::default();
        let uuid = block_on(engine.create_pool(
            name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            &EncryptionInfo::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        assert_eq!(
            block_on(engine.rename_pool(uuid, name)).unwrap(),
            RenameAction::Identity
        );
    }

    #[test]
    /// Renaming a pool to another pool should work if new name not taken
    fn rename_happens() {
        let engine = SimEngine::default();
        let uuid = block_on(engine.create_pool(
            "old_name",
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            &EncryptionInfo::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        assert_eq!(
            block_on(engine.rename_pool(uuid, "new_name")).unwrap(),
            RenameAction::Renamed(uuid)
        );
    }

    #[test]
    /// Renaming a pool to another pool should fail if new name taken
    fn rename_fails() {
        let new_name = "new_name";
        let engine = SimEngine::default();
        let uuid = block_on(engine.create_pool(
            "old_name",
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            &EncryptionInfo::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        block_on(engine.create_pool(
            new_name,
            strs_to_paths!(["/dev/four", "/dev/five", "/dev/six"]),
            None,
            &EncryptionInfo::default(),
        ))
        .unwrap();
        assert_matches!(
            block_on(engine.rename_pool(uuid, new_name)),
            Err(StratisError::Engine(ErrorEnum::AlreadyExists, _))
        );
    }

    #[test]
    /// Renaming should succeed if old_name absent, new present
    fn rename_no_op() {
        let new_name = "new_name";
        let engine = SimEngine::default();
        block_on(engine.create_pool(
            new_name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            &EncryptionInfo::default(),
        ))
        .unwrap();
        assert_matches!(
            block_on(engine.rename_pool(PoolUuid::new_v4(), new_name)),
            Ok(RenameAction::NoSource)
        );
    }
}

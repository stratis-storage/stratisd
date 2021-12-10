// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{hash_map::RandomState, HashMap, HashSet},
    iter::FromIterator,
    path::Path,
};

use serde_json::{json, Value};

use crate::{
    engine::{
        engine::{Engine, Report},
        shared::{create_pool_idempotent_or_err, validate_name, validate_paths},
        sim_engine::{keys::SimKeyActions, pool::SimPool},
        structures::Table,
        types::{
            CreateAction, DeleteAction, DevUuid, EncryptionInfo, FilesystemUuid, LockedPoolInfo,
            Name, PoolUuid, RenameAction, ReportType, SetUnlockAction, StratFilesystemDiff,
            ThinPoolDiff, UdevEngineEvent, UnlockMethod,
        },
    },
    stratis::{StratisError, StratisResult},
};

#[derive(Debug, Default)]
pub struct SimEngine {
    pools: Table<PoolUuid, SimPool>,
    key_handler: SimKeyActions,
}

impl<'a> Into<Value> for &'a SimEngine {
    // Precondition: SimPool Into<Value> impl return value always pattern matches
    // Value::Object(_)
    fn into(self) -> Value {
        json!({
            "pools": Value::Array(
                self.pools.iter().map(|(name, uuid, pool)| {
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

impl Engine for SimEngine {
    type Pool = SimPool;
    type KeyActions = SimKeyActions;

    fn create_pool(
        &mut self,
        name: &str,
        blockdev_paths: &[&Path],
        encryption_info: Option<&EncryptionInfo>,
    ) -> StratisResult<CreateAction<PoolUuid>> {
        validate_name(name)?;

        validate_paths(blockdev_paths)?;

        if let Some(key_desc) = encryption_info.and_then(|ei| ei.key_description()) {
            if !self.key_handler.contains_key(key_desc) {
                return Err(StratisError::Msg(format!(
                    "Key {} was not found in the keyring",
                    key_desc.as_application_str()
                )));
            }
        }

        match self.pools.get_by_name(name) {
            Some((_, pool)) => create_pool_idempotent_or_err(pool, name, blockdev_paths),
            None => {
                if blockdev_paths.is_empty() {
                    Err(StratisError::Msg(
                        "At least one blockdev is required to create a pool.".to_string(),
                    ))
                } else {
                    let device_set: HashSet<_, RandomState> = HashSet::from_iter(blockdev_paths);
                    let devices = device_set.into_iter().cloned().collect::<Vec<&Path>>();

                    let (pool_uuid, pool) = SimPool::new(&devices, encryption_info);

                    self.pools
                        .insert(Name::new(name.to_owned()), pool_uuid, pool);

                    Ok(CreateAction::Created(pool_uuid))
                }
            }
        }
    }

    fn handle_event(&mut self, _event: &UdevEngineEvent) -> Option<(Name, PoolUuid, &Self::Pool)> {
        None
    }

    fn destroy_pool(&mut self, uuid: PoolUuid) -> StratisResult<DeleteAction<PoolUuid>> {
        if let Some((_, pool)) = self.pools.get_by_uuid(uuid) {
            if pool.has_filesystems() {
                return Err(StratisError::Msg("filesystems remaining on pool".into()));
            };
        } else {
            return Ok(DeleteAction::Identity);
        }
        self.pools
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pool.get_by_uuid() returned a value")
            .1
            .destroy()?;
        Ok(DeleteAction::Deleted(uuid))
    }

    fn rename_pool(
        &mut self,
        uuid: PoolUuid,
        new_name: &str,
    ) -> StratisResult<RenameAction<PoolUuid>> {
        rename_pool_pre_idem!(self; uuid; new_name);

        let (_, pool) = self
            .pools
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pools.get_by_uuid() returned a value");

        self.pools
            .insert(Name::new(new_name.to_owned()), uuid, pool);
        Ok(RenameAction::Renamed(uuid))
    }

    fn unlock_pool(
        &mut self,
        _pool_uuid: PoolUuid,
        _unlock_method: UnlockMethod,
    ) -> StratisResult<SetUnlockAction<DevUuid>> {
        Ok(SetUnlockAction::empty())
    }

    fn get_pool(&self, uuid: PoolUuid) -> Option<(Name, &Self::Pool)> {
        get_pool!(self; uuid)
    }

    fn get_mut_pool(&mut self, uuid: PoolUuid) -> Option<(Name, &mut Self::Pool)> {
        get_mut_pool!(self; uuid)
    }

    fn locked_pools(&self) -> HashMap<PoolUuid, LockedPoolInfo> {
        HashMap::new()
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
        Ok(Vec::new())
    }

    fn pool_evented(
        &mut self,
        _: Option<&Vec<PoolUuid>>,
    ) -> StratisResult<HashMap<PoolUuid, ThinPoolDiff>> {
        Ok(HashMap::default())
    }

    fn fs_evented(
        &mut self,
        _: Option<&Vec<PoolUuid>>,
    ) -> StratisResult<HashMap<FilesystemUuid, StratFilesystemDiff>> {
        Ok(HashMap::default())
    }

    fn get_key_handler(&self) -> &Self::KeyActions {
        &self.key_handler
    }

    fn get_key_handler_mut(&mut self) -> &mut Self::KeyActions {
        &mut self.key_handler
    }

    fn is_sim(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {

    use std::{self, path::Path};

    use crate::engine::{
        engine::{Engine, Pool},
        types::{EngineAction, RenameAction},
    };

    use super::*;

    #[test]
    /// When an engine has no pools, any name lookup should fail
    fn get_pool_err() {
        assert_matches!(SimEngine::default().get_pool(PoolUuid::new_v4()), None);
    }

    #[test]
    /// When an engine has no pools, destroying any pool must succeed
    fn destroy_pool_empty() {
        assert_matches!(SimEngine::default().destroy_pool(PoolUuid::new_v4()), Ok(_));
    }

    #[test]
    /// Destroying an empty pool should succeed.
    fn destroy_empty_pool() {
        let mut engine = SimEngine::default();
        let uuid = engine
            .create_pool(
                "name",
                strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
                None,
            )
            .unwrap()
            .changed()
            .unwrap();
        assert_matches!(engine.destroy_pool(uuid), Ok(_));
    }

    #[test]
    /// Destroying a pool with devices should succeed
    fn destroy_pool_w_devices() {
        let mut engine = SimEngine::default();
        let uuid = engine
            .create_pool("name", strs_to_paths!(["/s/d"]), None)
            .unwrap()
            .changed()
            .unwrap();
        assert_matches!(engine.destroy_pool(uuid), Ok(_));
    }

    #[test]
    /// Destroying a pool with filesystems should fail
    fn destroy_pool_w_filesystem() {
        let mut engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = engine
            .create_pool(pool_name, strs_to_paths!(["/s/d"]), None)
            .unwrap()
            .changed()
            .unwrap();
        {
            let pool = engine.get_mut_pool(uuid).unwrap().1;
            pool.create_filesystems(pool_name, uuid, &[("test", None)])
                .unwrap();
        }
        assert_matches!(engine.destroy_pool(uuid), Err(_));
    }

    #[test]
    /// Creating a new pool with the same name and arguments should return
    /// identity.
    fn create_pool_name_collision() {
        let name = "name";
        let mut engine = SimEngine::default();
        let devices = strs_to_paths!(["/s/d"]);
        engine.create_pool(name, devices, None).unwrap();
        assert_matches!(
            engine.create_pool(name, devices, None),
            Ok(CreateAction::Identity)
        );
    }

    #[test]
    /// Creating a new pool with the same name and different arguments should fail
    fn create_pool_name_collision_different_args() {
        let name = "name";
        let mut engine = SimEngine::default();
        engine
            .create_pool(name, strs_to_paths!(["/s/d"]), None)
            .unwrap();
        assert_matches!(
            engine.create_pool(
                name,
                strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
                None,
            ),
            Err(_)
        );
    }

    #[test]
    /// Creating a pool with duplicate devices should succeed
    fn create_pool_duplicate_devices() {
        let path = "/s/d";
        let mut engine = SimEngine::default();
        assert_matches!(
            engine
                .create_pool("name", strs_to_paths!([path, path]), None)
                .unwrap()
                .changed()
                .map(|uuid| engine.get_pool(uuid).unwrap().1.blockdevs().len()),
            Some(1)
        );
    }

    #[test]
    /// Renaming a pool on an empty engine always works
    fn rename_empty() {
        let mut engine = SimEngine::default();
        assert_matches!(
            engine.rename_pool(PoolUuid::new_v4(), "new_name"),
            Ok(RenameAction::NoSource)
        );
    }

    #[test]
    /// Renaming a pool to itself always works
    fn rename_identity() {
        let name = "name";
        let mut engine = SimEngine::default();
        let uuid = engine
            .create_pool(
                name,
                strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
                None,
            )
            .unwrap()
            .changed()
            .unwrap();
        assert_eq!(
            engine.rename_pool(uuid, name).unwrap(),
            RenameAction::Identity
        );
    }

    #[test]
    /// Renaming a pool to another pool should work if new name not taken
    fn rename_happens() {
        let mut engine = SimEngine::default();
        let uuid = engine
            .create_pool(
                "old_name",
                strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
                None,
            )
            .unwrap()
            .changed()
            .unwrap();
        assert_eq!(
            engine.rename_pool(uuid, "new_name").unwrap(),
            RenameAction::Renamed(uuid)
        );
    }

    #[test]
    /// Renaming a pool to another pool should fail if new name taken
    fn rename_fails() {
        let new_name = "new_name";
        let mut engine = SimEngine::default();
        let uuid = engine
            .create_pool(
                "old_name",
                strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
                None,
            )
            .unwrap()
            .changed()
            .unwrap();
        engine
            .create_pool(
                new_name,
                strs_to_paths!(["/dev/four", "/dev/five", "/dev/six"]),
                None,
            )
            .unwrap();
        assert_matches!(engine.rename_pool(uuid, new_name), Err(_));
    }

    #[test]
    /// Renaming should succeed if old_name absent, new present
    fn rename_no_op() {
        let new_name = "new_name";
        let mut engine = SimEngine::default();
        engine
            .create_pool(
                new_name,
                strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
                None,
            )
            .unwrap();
        assert_matches!(
            engine.rename_pool(PoolUuid::new_v4(), new_name),
            Ok(RenameAction::NoSource)
        );
    }
}

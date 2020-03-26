// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    cell::RefCell,
    collections::{hash_map::RandomState, HashSet},
    iter::FromIterator,
    path::Path,
    rc::Rc,
};

use crate::{
    engine::{
        engine::{Engine, Eventable, Pool},
        shared::create_pool_idempotent_or_err,
        sim_engine::{pool::SimPool, randomization::Randomizer},
        structures::Table,
        types::{CreateAction, DeleteAction, Name, PoolUuid, RenameAction},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

#[derive(Debug, Default)]
pub struct SimEngine {
    pools: Table<SimPool>,
    rdm: Rc<RefCell<Randomizer>>,
}

impl SimEngine {}

impl Engine for SimEngine {
    fn create_pool(
        &mut self,
        name: &str,
        blockdev_paths: &[&Path],
        redundancy: Option<u16>,
        key_desc: Option<String>,
    ) -> StratisResult<CreateAction<PoolUuid>> {
        if key_desc.is_some() {
            return Err(StratisError::Error(
                "Key description parameters not currently accepted".to_string(),
            ));
        }

        let redundancy = calculate_redundancy!(redundancy);

        match self.pools.get_by_name(name) {
            Some((_, pool)) => create_pool_idempotent_or_err(pool, name, blockdev_paths),
            None => {
                if blockdev_paths.is_empty() {
                    Err(StratisError::Engine(
                        ErrorEnum::Invalid,
                        "At least one blockdev is required to create a pool.".to_string(),
                    ))
                } else {
                    let device_set: HashSet<_, RandomState> = HashSet::from_iter(blockdev_paths);
                    let devices = device_set.into_iter().cloned().collect::<Vec<&Path>>();

                    let (pool_uuid, pool) =
                        SimPool::new(&Rc::clone(&self.rdm), &devices, redundancy, key_desc);

                    if self.rdm.borrow_mut().throw_die() {
                        return Err(StratisError::Engine(ErrorEnum::Error, "X".into()));
                    }

                    self.pools
                        .insert(Name::new(name.to_owned()), pool_uuid, pool);

                    Ok(CreateAction::Created(pool_uuid))
                }
            }
        }
    }

    fn handle_event(&mut self, _event: &libudev::Event) -> Option<(PoolUuid, &mut dyn Pool)> {
        None
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

    fn get_pool(&self, uuid: PoolUuid) -> Option<(Name, &dyn Pool)> {
        get_pool!(self; uuid)
    }

    fn get_mut_pool(&mut self, uuid: PoolUuid) -> Option<(Name, &mut dyn Pool)> {
        get_mut_pool!(self; uuid)
    }

    /// Set properties of the simulator
    fn configure_simulator(&mut self, denominator: u32) -> StratisResult<()> {
        self.rdm.borrow_mut().set_probability(denominator);
        Ok(())
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
        None
    }

    fn evented(&mut self) -> StratisResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use std::{self, path::Path};

    use proptest::prelude::any;
    use uuid::Uuid;

    use crate::{
        engine::{
            types::{EngineAction, RenameAction},
            Engine,
        },
        stratis::{ErrorEnum, StratisError},
    };

    use super::*;

    proptest! {
        #[test]
        /// This method should always return Ok.
        fn configure_simulator_runs(denominator in any::<u32>()) {
            prop_assert!(SimEngine::default()
                .configure_simulator(denominator)
                .is_ok())
        }

    }

    #[test]
    /// When an engine has no pools, any name lookup should fail
    fn get_pool_err() {
        assert_matches!(SimEngine::default().get_pool(Uuid::new_v4()), None);
    }

    #[test]
    /// When an engine has no pools, destroying any pool must succeed
    fn destroy_pool_empty() {
        assert_matches!(SimEngine::default().destroy_pool(Uuid::new_v4()), Ok(_));
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
            .create_pool("name", strs_to_paths!(["/s/d"]), None, None)
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
            .create_pool(pool_name, strs_to_paths!(["/s/d"]), None, None)
            .unwrap()
            .changed()
            .unwrap();
        {
            let pool = engine.get_mut_pool(uuid).unwrap().1;
            pool.create_filesystems(uuid, pool_name, &[("test", None)])
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
        engine.create_pool(name, devices, None, None).unwrap();
        assert_matches!(
            engine.create_pool(name, devices, None, None),
            Ok(CreateAction::Identity)
        );
    }

    #[test]
    /// Creating a new pool with the same name and different arguments should fail
    fn create_pool_name_collision_different_args() {
        let name = "name";
        let mut engine = SimEngine::default();
        engine
            .create_pool(name, strs_to_paths!(["/s/d"]), None, None)
            .unwrap();
        assert_matches!(
            engine.create_pool(
                name,
                strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
                None,
                None,
            ),
            Err(StratisError::Engine(ErrorEnum::Invalid, _))
        );
    }

    #[test]
    /// Creating a pool with duplicate devices should succeed
    fn create_pool_duplicate_devices() {
        let path = "/s/d";
        let mut engine = SimEngine::default();
        assert_matches!(
            engine
                .create_pool("name", strs_to_paths!([path, path]), None, None)
                .unwrap()
                .changed()
                .map(|uuid| engine.get_pool(uuid).unwrap().1.blockdevs().len()),
            Some(1)
        );
    }

    #[test]
    /// Creating a pool with an impossible raid level should fail
    fn create_pool_max_u16_raid() {
        let mut engine = SimEngine::default();
        assert_matches!(
            engine.create_pool(
                "name",
                strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
                Some(std::u16::MAX),
                None,
            ),
            Err(_)
        );
    }

    #[test]
    /// Renaming a pool on an empty engine always works
    fn rename_empty() {
        let mut engine = SimEngine::default();
        assert_matches!(
            engine.rename_pool(Uuid::new_v4(), "new_name"),
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
                None,
            )
            .unwrap();
        assert_matches!(
            engine.rename_pool(uuid, new_name),
            Err(StratisError::Engine(ErrorEnum::AlreadyExists, _))
        );
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
                None,
            )
            .unwrap();
        assert_matches!(
            engine.rename_pool(Uuid::new_v4(), new_name),
            Ok(RenameAction::NoSource)
        );
    }
}

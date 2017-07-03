// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cell::RefCell;
use std::collections::HashSet;
use std::collections::hash_map::RandomState;
use std::iter::FromIterator;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use super::super::engine::{Engine, HasName, HasUuid, Pool};
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::structures::Table;
use super::super::types::{PoolUuid, Redundancy, RenameAction};

use super::pool::SimPool;
use super::randomization::Randomizer;


#[derive(Debug, Default)]
pub struct SimEngine {
    pools: Table<SimPool>,
    rdm: Rc<RefCell<Randomizer>>,
}

impl SimEngine {}

impl Engine for SimEngine {
    fn create_pool(&mut self,
                   name: &str,
                   blockdev_paths: &[&Path],
                   redundancy: Option<u16>,
                   _force: bool)
                   -> EngineResult<(PoolUuid, Vec<PathBuf>)> {

        let redundancy = calculate_redundancy!(redundancy);

        if self.pools.contains_name(name) {
            return Err(EngineError::Engine(ErrorEnum::AlreadyExists, name.into()));
        }

        let device_set: HashSet<_, RandomState> = HashSet::from_iter(blockdev_paths);
        let devices = device_set
            .into_iter()
            .map(|x| *x)
            .collect::<Vec<&Path>>();

        let pool = SimPool::new(self.rdm.clone(), name, &devices, redundancy);

        if self.rdm.borrow_mut().throw_die() {
            return Err(EngineError::Engine(ErrorEnum::Error, "X".into()));
        }

        let bdev_paths = pool.block_devs
            .values()
            .map(|p| p.devnode.clone())
            .collect();
        let uuid = *pool.uuid();
        self.pools.insert(pool);

        Ok((uuid, bdev_paths))
    }

    fn destroy_pool(&mut self, uuid: &PoolUuid) -> EngineResult<bool> {
        destroy_pool!{self; uuid}
    }

    fn rename_pool(&mut self, uuid: &PoolUuid, new_name: &str) -> EngineResult<RenameAction> {
        rename_pool!{self; uuid; new_name}
    }

    fn get_pool(&mut self, uuid: &PoolUuid) -> Option<&mut Pool> {
        get_pool!(self; uuid)
    }

    /// Set properties of the simulator
    fn configure_simulator(&mut self, denominator: u32) -> EngineResult<()> {
        self.rdm.borrow_mut().set_probability(denominator);
        Ok(())
    }

    fn check(&mut self) -> () {
        check_engine!(self)
    }
}

#[cfg(test)]
mod tests {

    use std;
    use std::path::Path;

    use uuid::Uuid;

    use quickcheck::QuickCheck;

    use super::SimEngine;

    use engine::Engine;
    use engine::EngineError;
    use engine::ErrorEnum;
    use engine::RenameAction;

    #[test]
    fn prop_configure_simulator_runs() {

        /// Configure simulator should always return Ok.
        fn configure_simulator_runs(denominator: u32) -> bool {
            SimEngine::default()
                .configure_simulator(denominator)
                .is_ok()
        }

        QuickCheck::new()
            .tests(10)
            .quickcheck(configure_simulator_runs as fn(u32) -> bool);
    }

    #[test]
    /// When an engine has no pools, any name lookup should fail
    fn get_pool_err() {
        assert!(SimEngine::default().get_pool(&Uuid::new_v4()).is_none());
    }

    #[test]
    /// When an engine has no pools, destroying any pool must succeed
    fn destroy_pool_empty() {
        assert!(SimEngine::default()
                    .destroy_pool(&Uuid::new_v4())
                    .is_ok());
    }

    #[test]
    /// Destroying an empty pool should succeed.
    fn destroy_empty_pool() {
        let mut engine = SimEngine::default();
        let (uuid, _) = engine.create_pool("name", &[], None, false).unwrap();
        assert!(engine.destroy_pool(&uuid).is_ok());
    }

    #[test]
    /// Destroying a pool with devices should succeed
    fn destroy_pool_w_devices() {
        let mut engine = SimEngine::default();
        let (uuid, _) = engine
            .create_pool("name", &[Path::new("/s/d")], None, false)
            .unwrap();
        assert!(engine.destroy_pool(&uuid).is_ok());
    }

    #[test]
    /// Destroying a pool with filesystems should fail
    fn destroy_pool_w_filesystem() {
        let mut engine = SimEngine::default();
        let (uuid, _) = engine
            .create_pool("name", &[Path::new("/s/d")], None, false)
            .unwrap();
        {
            let pool = engine.get_pool(&uuid).unwrap();
            pool.create_filesystems(&["test"]).unwrap();
        }
        assert!(engine.destroy_pool(&uuid).is_err());
    }

    #[test]
    #[ignore]
    /// Creating a new pool identical to the previous should succeed
    fn create_new_pool_twice() {
        let name = "name";
        let mut engine = SimEngine::default();
        engine.create_pool(name, &[], None, false).unwrap();
        assert!(match engine.create_pool(name, &[], None, false) {
                    Ok((_, devs)) => devs.is_empty(),
                    Err(_) => false,
                });
    }

    #[test]
    /// Creating a new pool with the same name should fail
    fn create_pool_name_collision() {
        let name = "name";
        let mut engine = SimEngine::default();
        engine
            .create_pool(name, &[Path::new("/s/d")], None, false)
            .unwrap();
        assert!(match engine.create_pool(name, &[], None, false) {
                    Err(EngineError::Engine(ErrorEnum::AlreadyExists, _)) => true,
                    _ => false,
                });
    }

    #[test]
    /// Creating a pool with duplicate devices should succeed
    fn create_pool_duplicate_devices() {
        let path = "/s/d";
        let mut engine = SimEngine::default();
        let devices = vec![Path::new(path), Path::new(path)];
        assert!(match engine.create_pool("name", &devices, None, false) {
                    Ok((_, devs)) => devs.len() == 1,
                    _ => false,
                });
    }

    #[test]
    /// Creating a pool with an impossible raid level should fail
    fn create_pool_max_u16_raid() {
        let mut engine = SimEngine::default();
        assert!(engine
                    .create_pool("name", &[], Some(std::u16::MAX), false)
                    .is_err());
    }

    #[test]
    /// Renaming a pool on an empty engine always works
    fn rename_empty() {
        let mut engine = SimEngine::default();
        assert!(match engine.rename_pool(&Uuid::new_v4(), "new_name") {
                    Ok(RenameAction::NoSource) => true,
                    _ => false,
                });
    }

    #[test]
    /// Renaming a pool to itself always works
    fn rename_identity() {
        let name = "name";
        let mut engine = SimEngine::default();
        let (uuid, _) = engine.create_pool(name, &[], None, false).unwrap();
        assert!(match engine.rename_pool(&uuid, name) {
                    Ok(RenameAction::Identity) => true,
                    _ => false,
                });
    }

    #[test]
    /// Renaming a pool to another pool should work if new name not taken
    fn rename_happens() {
        let mut engine = SimEngine::default();
        let (uuid, _) = engine.create_pool("old_name", &[], None, false).unwrap();
        assert!(match engine.rename_pool(&uuid, "new_name") {
                    Ok(RenameAction::Renamed) => true,
                    _ => false,
                });
    }

    #[test]
    /// Renaming a pool to another pool should fail if new name taken
    fn rename_fails() {
        let new_name = "new_name";
        let mut engine = SimEngine::default();
        let (uuid, _) = engine.create_pool("old_name", &[], None, false).unwrap();
        engine.create_pool(new_name, &[], None, false).unwrap();
        assert!(match engine.rename_pool(&uuid, new_name) {
                    Err(EngineError::Engine(ErrorEnum::AlreadyExists, _)) => true,
                    _ => false,
                });
    }

    #[test]
    /// Renaming should succeed if old_name absent, new present
    fn rename_no_op() {
        let new_name = "new_name";
        let mut engine = SimEngine::default();
        engine.create_pool(new_name, &[], None, false).unwrap();
        assert!(match engine.rename_pool(&Uuid::new_v4(), new_name) {
                    Ok(RenameAction::NoSource) => true,
                    _ => false,
                });
    }

}

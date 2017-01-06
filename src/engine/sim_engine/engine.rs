// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use engine::Engine;
use engine::EngineError;
use engine::EngineResult;
use engine::ErrorEnum;

use engine::Pool;
use engine::RenameAction;
use engine::Redundancy;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::collections::BTreeSet;
use std::iter::FromIterator;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use super::pool::SimPool;
use super::randomization::Randomizer;



#[derive(Debug)]
pub struct SimEngine {
    pools: BTreeMap<String, SimPool>,
    rdm: Rc<RefCell<Randomizer>>,
}

impl SimEngine {
    pub fn new() -> SimEngine {
        SimEngine {
            pools: BTreeMap::new(),
            rdm: Rc::new(RefCell::new(Randomizer::new())),
        }
    }
}

impl Engine for SimEngine {
    fn create_pool(&mut self,
                   name: &str,
                   blockdev_paths: &[&Path],
                   redundancy: Option<u16>,
                   _force: bool)
                   -> EngineResult<Vec<PathBuf>> {

        let redundancy = calculate_redundancy!(redundancy);

        if self.pools.contains_key(name) {
            return Err(EngineError::Stratis(ErrorEnum::AlreadyExists(name.into())));
        }

        let devices =
            BTreeSet::from_iter(blockdev_paths).into_iter().map(|x| *x).collect::<Vec<&Path>>();

        let pool = SimPool::new(self.rdm.clone(), &devices, redundancy);

        if self.rdm.borrow_mut().throw_die() {
            return Err(EngineError::Stratis(ErrorEnum::Error("X".into())));
        }

        let bdev_paths = pool.block_devs.values().map(|p| p.devnode.clone()).collect();
        self.pools.insert(name.to_owned(), pool);

        Ok(bdev_paths)
    }

    fn destroy_pool(&mut self, name: &str) -> EngineResult<bool> {
        destroy_pool!{self; name}
    }

    fn rename_pool(&mut self, old_name: &str, new_name: &str) -> EngineResult<RenameAction> {
        rename_pool!{self; old_name; new_name}
    }

    /// Looks up the pool by its unique name
    fn get_pool(&mut self, name: &str) -> EngineResult<&mut Pool> {
        get_pool!(self; name)
    }

    /// Returns a collection of the Pool objects that belong to this engine
    fn pools(&mut self) -> BTreeMap<&str, &mut Pool> {
        pools!(self)
    }

    /// Set properties of the simulator
    fn configure_simulator(&mut self, denominator: u32) -> EngineResult<()> {
        self.rdm.borrow_mut().set_probability(denominator);
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use std::path::Path;

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
            SimEngine::new().configure_simulator(denominator).is_ok()
        }

        QuickCheck::new().tests(10).quickcheck(configure_simulator_runs as fn(u32) -> bool);
    }

    #[test]
    /// When an engine has no pools, any name lookup should fail
    fn get_pool_err() {
        assert!(match SimEngine::new().get_pool("name") {
            Err(EngineError::Stratis(ErrorEnum::NotFound(_))) => true,
            _ => false,
        });
    }

    #[test]
    /// When an engine has no pools, the thing returned by pools() is empty
    fn pools_empty() {
        assert!(SimEngine::new().pools().is_empty());
    }

    #[test]
    /// When an engine has no pools, destroying any pool must succeed
    fn destroy_pool_empty() {
        assert!(SimEngine::new().destroy_pool("name").is_ok());
    }

    #[test]
    /// Destroying an empty pool should succeed.
    fn destroy_empty_pool() {
        let name = "name";
        let mut engine = SimEngine::new();
        engine.create_pool(name, &[], None, false).unwrap();
        assert!(engine.destroy_pool(name).is_ok());
    }

    #[test]
    /// Destroying a pool with devices should fail
    fn destroy_pool_w_devices() {
        let name = "name";
        let mut engine = SimEngine::new();
        engine.create_pool(name, &[Path::new("/s/d")], None, false).unwrap();
        assert!(match engine.destroy_pool(name) {
            Err(EngineError::Stratis(ErrorEnum::Busy(_))) => true,
            _ => false,
        });
    }

    #[test]
    #[ignore]
    /// Creating a new pool identical to the previous should succeed
    fn create_new_pool_twice() {
        let name = "name";
        let mut engine = SimEngine::new();
        engine.create_pool(name, &[], None, false).unwrap();
        assert!(match engine.create_pool(name, &[], None, false) {
            Ok(devs) => devs.is_empty(),
            Err(_) => false,
        });
    }

    #[test]
    /// Creating a new pool with the same name should fail
    fn create_pool_name_collision() {
        let name = "name";
        let mut engine = SimEngine::new();
        engine.create_pool(name, &[Path::new("/s/d")], None, false).unwrap();
        assert!(match engine.create_pool(name, &[], None, false) {
            Err(EngineError::Stratis(ErrorEnum::AlreadyExists(_))) => true,
            _ => false,
        });
    }

    #[test]
    /// Creating a pool with duplicate devices should succeed
    fn create_pool_duplicate_devices() {
        let path = "/s/d";
        let mut engine = SimEngine::new();
        let devices = vec![Path::new(path), Path::new(path)];
        assert!(match engine.create_pool("name", &devices, None, false) {
            Ok(devs) => devs.len() == 1,
            _ => false,
        });
    }

    #[test]
    /// Creating a pool with an impossible raid level should fail
    fn create_pool_max_u16_raid() {
        let mut engine = SimEngine::new();
        assert!(engine.create_pool("name", &[], Some(u16::max_value()), false).is_err());
    }

    #[test]
    /// Renaming a pool on an empty engine always works
    fn rename_empty() {
        let mut engine = SimEngine::new();
        assert!(match engine.rename_pool("old_name", "new_name") {
            Ok(RenameAction::NoSource) => true,
            _ => false,
        });
    }

    #[test]
    /// Renaming a pool to itself in an empty engine always works
    fn rename_empty_identity() {
        let mut engine = SimEngine::new();
        assert!(match engine.rename_pool("old_name", "old_name") {
            Ok(RenameAction::Identity) => true,
            _ => false,
        });
    }

    #[test]
    /// Renaming a pool to itself always works
    fn rename_identity() {
        let name = "name";
        let mut engine = SimEngine::new();
        engine.create_pool(name, &[], None, false).unwrap();
        assert!(match engine.rename_pool(name, name) {
            Ok(RenameAction::Identity) => true,
            _ => false,
        });
    }

    #[test]
    /// Renaming a pool to another pool should work if new name not taken
    fn rename_happens() {
        let name = "old_name";
        let mut engine = SimEngine::new();
        engine.create_pool(name, &[], None, false).unwrap();
        assert!(match engine.rename_pool(name, "new_name") {
            Ok(RenameAction::Renamed) => true,
            _ => false,
        });
    }

    #[test]
    /// Renaming a pool to another pool should fail if new name taken
    fn rename_fails() {
        let old_name = "old_name";
        let new_name = "new_name";
        let mut engine = SimEngine::new();
        engine.create_pool(old_name, &[], None, false).unwrap();
        engine.create_pool(new_name, &[], None, false).unwrap();
        assert!(match engine.rename_pool(old_name, new_name) {
            Err(EngineError::Stratis(ErrorEnum::AlreadyExists(_))) => true,
            _ => false,
        });
    }

    #[test]
    /// Renaming should succeed if old_name absent, new present
    fn rename_no_op() {
        let old_name = "old_name";
        let new_name = "new_name";
        let mut engine = SimEngine::new();
        engine.create_pool(new_name, &[], None, false).unwrap();
        assert!(match engine.rename_pool(old_name, new_name) {
            Ok(RenameAction::NoSource) => true,
            _ => false,
        });
    }

}

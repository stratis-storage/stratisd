// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use engine::Engine;
use engine::EngineError;
use engine::EngineResult;
use engine::ErrorEnum;

use engine::Pool;

use std::cell::RefCell;
use std::path::Path;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::iter::FromIterator;
use std::rc::Rc;

use super::blockdev::SimDev;
use super::pool::SimPool;
use super::randomization::Randomizer;



#[derive(Debug)]
pub struct SimEngine {
    pub pools: BTreeMap<String, SimPool>,
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
                   raid_level: u16,
                   _force: bool)
                   -> EngineResult<usize> {

        if self.pools.contains_key(name) {
            return Err(EngineError::Stratis(ErrorEnum::AlreadyExists(name.into())));
        }

        let mut devs: Vec<SimDev> =
            blockdev_paths.iter().map(|x| SimDev::new_dev(self.rdm.clone(), x)).collect();

        for dev in devs.iter_mut() {
            dev.update();
        }

        let bad_devs: Vec<&SimDev> = devs.iter().filter(|dev| !dev.usable()).collect();

        if !bad_devs.is_empty() {
            return Err(EngineError::Stratis(ErrorEnum::Busy("some devices are unusable".into())));
        }

        let pool = SimPool::new_pool(self.rdm.clone(), devs.as_slice(), raid_level);

        if self.rdm.borrow_mut().throw_die() {
            return Err(EngineError::Stratis(ErrorEnum::Error("X".into())));
        }

        let num_bdevs = pool.block_devs.len();
        self.pools.insert(name.to_owned(), pool);

        Ok(num_bdevs)
    }

    /// Destroy a pool, if the pool does not exist, return Ok.
    fn destroy_pool(&mut self, name: &str) -> EngineResult<()> {
        destroy_pool!{self; name}
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
        engine.create_pool(name, &vec![], 0, false).unwrap();
        assert!(engine.destroy_pool(name).is_ok());
    }

    #[test]
    /// Destroying a pool with devices should fail
    fn destroy_pool_w_devices() {
        let name = "name";
        let mut engine = SimEngine::new();
        engine.create_pool(name, &vec![Path::new("/s/d")], 0, false).unwrap();
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
        engine.create_pool(name, &vec![], 0, false).unwrap();
        assert!(engine.create_pool(name, &vec![], 0, false).is_ok());
    }

    #[test]
    /// Creating a new pool with the same name should fail
    fn create_pool_name_collision() {
        let name = "name";
        let mut engine = SimEngine::new();
        engine.create_pool(name, &vec![Path::new("/s/d")], 0, false).unwrap();
        assert!(match engine.create_pool(name, &vec![], 0, false) {
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
        assert!(engine.create_pool("name", &devices, 0, false).is_ok());
    }

    #[test]
    #[ignore]
    /// Creating a pool with an impossible raid level should fail
    fn create_pool_max_u16_raid() {
        let mut engine = SimEngine::new();
        assert!(engine.create_pool("name", &vec![], u16::max_value(), false).is_err());
    }

}

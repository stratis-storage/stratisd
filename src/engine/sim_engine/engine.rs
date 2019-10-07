// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    cell::RefCell,
    collections::{hash_map::RandomState, HashSet},
    iter::FromIterator,
    path::{Path, PathBuf},
    rc::Rc,
};

use devicemapper::Device;

use crate::{
    engine::{
        engine::{Engine, Eventable, Pool},
        sim_engine::{pool::SimPool, randomization::Randomizer},
        structures::{Table, Threaded},
        types::{Name, PoolUuid, Redundancy, RenameAction},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

#[derive(Debug, Default)]
pub struct SimEngine {
    pools: Table<Threaded<SimPool>>,
    rdm: Rc<RefCell<Randomizer>>,
}

impl SimEngine {}

impl Engine for SimEngine {
    fn create_pool(
        &mut self,
        name: &str,
        blockdev_paths: &[&Path],
        redundancy: Option<u16>,
    ) -> StratisResult<PoolUuid> {
        let redundancy = calculate_redundancy!(redundancy);

        if self.pools.contains_name(name) {
            return Err(StratisError::Engine(ErrorEnum::AlreadyExists, name.into()));
        }

        let device_set: HashSet<_, RandomState> = HashSet::from_iter(blockdev_paths);
        let devices = device_set.into_iter().cloned().collect::<Vec<&Path>>();

        let (pool_uuid, pool) = SimPool::new(&Rc::clone(&self.rdm), &devices, redundancy);

        if self.rdm.borrow_mut().throw_die() {
            return Err(StratisError::Engine(ErrorEnum::Error, "X".into()));
        }

        self.pools
            .insert(Name::new(name.to_owned()), pool_uuid, Threaded::new(pool));

        Ok(pool_uuid)
    }

    fn block_evaluate(
        &mut self,
        device: Device,
        dev_node: PathBuf,
    ) -> StratisResult<Option<PoolUuid>> {
        assert_ne!(dev_node, PathBuf::from("/"));
        assert_ne!(libc::dev_t::from(device), 0);
        Ok(None)
    }

    fn destroy_pool(&mut self, uuid: PoolUuid) -> StratisResult<bool> {
        if let Some((_, pool)) = self.pools.get_by_uuid(uuid) {
            if pool.read_map(|p| p.has_filesystems())? {
                return Err(StratisError::Engine(
                    ErrorEnum::Busy,
                    "filesystems remaining on pool".into(),
                ));
            } else {

            }
        } else {
            return Ok(false);
        }
        self.pools
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pool.get_by_uuid() returned a value")
            .1
            .write_and_then(|p| p.destroy())?;
        Ok(true)
    }

    fn rename_pool(&mut self, uuid: PoolUuid, new_name: &str) -> StratisResult<RenameAction> {
        rename_pool_pre!(self; uuid; new_name);

        let (_, pool) = self
            .pools
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pools.get_by_uuid() returned a value");

        self.pools
            .insert(Name::new(new_name.to_owned()), uuid, pool);
        Ok(RenameAction::Renamed)
    }

    fn get_pool(&self, uuid: PoolUuid) -> Option<(Name, Threaded<dyn Pool>)> {
        get_pool!(self; uuid)
    }

    /// Set properties of the simulator
    fn configure_simulator(&mut self, denominator: u32) -> StratisResult<()> {
        self.rdm.borrow_mut().set_probability(denominator);
        Ok(())
    }

    fn pools(&self) -> Vec<(Name, PoolUuid, Threaded<dyn Pool>)> {
        self.pools
            .iter()
            .map(|(name, uuid, pool)| (name.clone(), *uuid, Threaded::from(pool)))
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
        engine::{Engine, RenameAction},
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
        let uuid = engine.create_pool("name", &[], None).unwrap();
        assert_matches!(engine.destroy_pool(uuid), Ok(_));
    }

    #[test]
    /// Destroying a pool with devices should succeed
    fn destroy_pool_w_devices() {
        let mut engine = SimEngine::default();
        let uuid = engine
            .create_pool("name", &[Path::new("/s/d")], None)
            .unwrap();
        assert_matches!(engine.destroy_pool(uuid), Ok(_));
    }

    #[test]
    /// Destroying a pool with filesystems should fail
    fn destroy_pool_w_filesystem() {
        let mut engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = engine
            .create_pool(pool_name, &[Path::new("/s/d")], None)
            .unwrap();
        {
            let pool = engine.get_pool(uuid).unwrap().1;
            pool.write_and_then(|p| p.create_filesystems(uuid, pool_name, &[("test", None)]))
                .unwrap();
        }
        assert_matches!(engine.destroy_pool(uuid), Err(_));
    }

    #[test]
    #[ignore]
    /// Creating a new pool identical to the previous should succeed
    fn create_new_pool_twice() {
        let name = "name";
        let mut engine = SimEngine::default();
        engine.create_pool(name, &[], None).unwrap();
        assert!(match engine.create_pool(name, &[], None) {
            Ok(uuid) => engine
                .get_pool(uuid)
                .unwrap()
                .1
                .read_map(|p| p.blockdevs().is_empty())
                .unwrap(),
            Err(_) => false,
        });
    }

    #[test]
    /// Creating a new pool with the same name should fail
    fn create_pool_name_collision() {
        let name = "name";
        let mut engine = SimEngine::default();
        engine
            .create_pool(name, &[Path::new("/s/d")], None)
            .unwrap();
        assert_matches!(
            engine.create_pool(name, &[], None),
            Err(StratisError::Engine(ErrorEnum::AlreadyExists, _))
        );
    }

    #[test]
    /// Creating a pool with duplicate devices should succeed
    fn create_pool_duplicate_devices() {
        let path = "/s/d";
        let mut engine = SimEngine::default();
        let devices = vec![Path::new(path), Path::new(path)];
        assert_matches!(
            engine.create_pool("name", &devices, None).map(|uuid| engine
                .get_pool(uuid)
                .unwrap()
                .1
                .read_map(|p| p.blockdevs().len())
                .unwrap()),
            Ok(1)
        );
    }

    #[test]
    /// Creating a pool with an impossible raid level should fail
    fn create_pool_max_u16_raid() {
        let mut engine = SimEngine::default();
        assert_matches!(engine.create_pool("name", &[], Some(std::u16::MAX)), Err(_));
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
        let uuid = engine.create_pool(name, &[], None).unwrap();
        assert_matches!(engine.rename_pool(uuid, name), Ok(RenameAction::Identity));
    }

    #[test]
    /// Renaming a pool to another pool should work if new name not taken
    fn rename_happens() {
        let mut engine = SimEngine::default();
        let uuid = engine.create_pool("old_name", &[], None).unwrap();
        assert_matches!(
            engine.rename_pool(uuid, "new_name"),
            Ok(RenameAction::Renamed)
        );
    }

    #[test]
    /// Renaming a pool to another pool should fail if new name taken
    fn rename_fails() {
        let new_name = "new_name";
        let mut engine = SimEngine::default();
        let uuid = engine.create_pool("old_name", &[], None).unwrap();
        engine.create_pool(new_name, &[], None).unwrap();
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
        engine.create_pool(new_name, &[], None).unwrap();
        assert_matches!(
            engine.rename_pool(Uuid::new_v4(), new_name),
            Ok(RenameAction::NoSource)
        );
    }

}

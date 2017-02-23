// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::iter::FromIterator;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::vec::Vec;

use uuid::Uuid;

use engine::EngineError;
use engine::EngineResult;
use engine::ErrorEnum;
use engine::Filesystem;
use engine::Pool;
use engine::RenameAction;

use engine::engine::Redundancy;

use super::blockdev::SimDev;
use super::super::engine::{HasName, HasUuid};
use super::super::structures::Table;
use super::filesystem::SimFilesystem;
use super::randomization::Randomizer;

#[derive(Debug)]
pub struct SimPool {
    name: String,
    pool_uuid: Uuid,
    pub block_devs: BTreeMap<PathBuf, SimDev>,
    pub cache_devs: BTreeMap<PathBuf, SimDev>,
    pub filesystems: Table<SimFilesystem>,
    redundancy: Redundancy,
    rdm: Rc<RefCell<Randomizer>>,
}

impl SimPool {
    pub fn new(rdm: Rc<RefCell<Randomizer>>,
               name: &str,
               paths: &[&Path],
               redundancy: Redundancy)
               -> SimPool {

        let devices = BTreeSet::from_iter(paths);
        let device_pairs = devices.iter()
            .map(|p| (p.to_path_buf(), SimDev::new(rdm.clone(), p)));
        let new_pool = SimPool {
            name: name.to_owned(),
            pool_uuid: Uuid::new_v4(),
            block_devs: BTreeMap::from_iter(device_pairs),
            filesystems: Table::new(),
            cache_devs: BTreeMap::new(),
            redundancy: redundancy,
            rdm: rdm.clone(),
        };

        new_pool
    }
}

impl Pool for SimPool {
    fn add_blockdevs(&mut self, paths: &[&Path], _force: bool) -> EngineResult<Vec<PathBuf>> {
        let rdm = self.rdm.clone();
        let devices = BTreeSet::from_iter(paths);
        let device_pairs = devices.iter()
            .map(|p| (p.to_path_buf(), SimDev::new(rdm.clone(), p)));
        self.block_devs.extend(device_pairs);
        Ok(devices.iter().map(|d| d.to_path_buf()).collect())
    }

    fn add_cachedevs(&mut self, paths: &[&Path], _force: bool) -> EngineResult<Vec<PathBuf>> {
        let rdm = self.rdm.clone();
        let devices = BTreeSet::from_iter(paths);
        let device_pairs = devices.iter()
            .map(|p| (p.to_path_buf(), SimDev::new(rdm.clone(), p)));
        self.cache_devs.extend(device_pairs);
        Ok(devices.iter().map(|d| d.to_path_buf()).collect())
    }

    fn destroy_filesystems<'a, 'b>(&'a mut self, fs_uuids: &'b [Uuid]) -> EngineResult<Vec<Uuid>> {
        destroy_filesystems!{self; fs_uuids}
    }

    fn destroy(self) -> EngineResult<()> {
        // Nothing to do here.
        Ok(())
    }

    fn create_filesystems<'a, 'b>(&'a mut self,
                                  specs: &[&'b str])
                                  -> EngineResult<Vec<(&'b str, Uuid)>> {
        let names = BTreeSet::from_iter(specs);
        for name in names.iter() {
            if self.filesystems.contains_name(name) {
                return Err(EngineError::Engine(ErrorEnum::AlreadyExists, name.to_string()));
            }
        }

        let mut result = Vec::new();
        for name in names.iter() {
            let uuid = Uuid::new_v4();
            let new_filesystem = SimFilesystem::new(uuid, name);
            self.filesystems.insert(new_filesystem);
            result.push((**name, uuid));
        }

        Ok(result)
    }

    fn create_snapshot<'a, 'b, 'c>(&'a mut self,
                                   _snapshot_name: &'b str,
                                   _source: &'c Uuid)
                                   -> EngineResult<Uuid> {
        unimplemented!()
    }

    fn rename_filesystem(&mut self, uuid: &Uuid, new_name: &str) -> EngineResult<RenameAction> {
        rename_filesystem!{self; uuid; new_name}
    }

    fn rename(&mut self, name: &str) {
        self.name = name.to_owned();
    }

    fn get_filesystem(&mut self, uuid: &Uuid) -> Option<&mut Filesystem> {
        get_filesystem!(self; uuid)
    }
}

impl HasUuid for SimPool {
    fn uuid(&self) -> &Uuid {
        &self.pool_uuid
    }
}

impl HasName for SimPool {
    fn name(&self) -> &str {
        &self.name
    }
}


#[cfg(test)]
mod tests {

    use std::path::Path;

    use uuid::Uuid;

    use engine::Engine;
    use engine::ErrorEnum;
    use engine::EngineError;
    use engine::RenameAction;

    use super::super::SimEngine;

    #[test]
    /// Renaming a filesystem on an empty pool always works
    fn rename_empty() {
        let mut engine = SimEngine::new();
        let (uuid, _) = engine.create_pool("name", &[], None, false).unwrap();
        let pool = engine.get_pool(&uuid).unwrap();
        assert!(match pool.rename_filesystem(&Uuid::new_v4(), "new_name") {
            Ok(RenameAction::NoSource) => true,
            _ => false,
        });
    }

    #[test]
    /// Renaming a filesystem to another filesystem should work if new name not taken
    fn rename_happens() {
        let mut engine = SimEngine::new();
        let (uuid, _) = engine.create_pool("name", &[], None, false).unwrap();
        let pool = engine.get_pool(&uuid).unwrap();
        let infos = pool.create_filesystems(&["old_name"]).unwrap();
        assert!(match pool.rename_filesystem(&infos[0].1, "new_name") {
            Ok(RenameAction::Renamed) => true,
            _ => false,
        });
    }

    #[test]
    /// Renaming a filesystem to another filesystem should fail if new name taken
    fn rename_fails() {
        let old_name = "old_name";
        let new_name = "new_name";
        let mut engine = SimEngine::new();
        let (uuid, _) = engine.create_pool("name", &[], None, false).unwrap();
        let pool = engine.get_pool(&uuid).unwrap();
        let results = pool.create_filesystems(&[old_name, new_name])
            .unwrap();
        let old_uuid = results.iter().find(|x| x.0 == old_name).unwrap().1;
        assert!(match pool.rename_filesystem(&old_uuid, new_name) {
            Err(EngineError::Engine(ErrorEnum::AlreadyExists, _)) => true,
            _ => false,
        });
    }

    #[test]
    /// Renaming should succeed if old_name absent, new present
    fn rename_no_op() {
        let new_name = "new_name";
        let mut engine = SimEngine::new();
        let (uuid, _) = engine.create_pool("name", &[], None, false).unwrap();
        let pool = engine.get_pool(&uuid).unwrap();
        pool.create_filesystems(&[new_name]).unwrap();
        assert!(match pool.rename_filesystem(&Uuid::new_v4(), new_name) {
            Ok(RenameAction::NoSource) => true,
            _ => false,
        });
    }

    #[test]
    /// Removing an empty list of filesystems should always succeed
    fn destroy_fs_empty() {
        let mut engine = SimEngine::new();
        let (uuid, _) = engine.create_pool("name", &[], None, false).unwrap();
        let mut pool = engine.get_pool(&uuid).unwrap();
        assert!(match pool.destroy_filesystems(&[]) {
            Ok(names) => names.is_empty(),
            _ => false,
        });
    }

    #[test]
    /// Removing a non-empty list of filesystems should succeed on empty pool
    fn destroy_fs_some() {
        let mut engine = SimEngine::new();
        let (uuid, _) = engine.create_pool("name", &[], None, false).unwrap();
        let mut pool = engine.get_pool(&uuid).unwrap();
        assert!(pool.destroy_filesystems(&[Uuid::new_v4()]).is_ok());
    }

    #[test]
    /// Removing a non-empty list of filesystems should succeed on any pool
    fn destroy_fs_any() {
        let mut engine = SimEngine::new();
        let (uuid, _) = engine.create_pool("name", &[], None, false).unwrap();
        let mut pool = engine.get_pool(&uuid).unwrap();
        let fs_results = pool.create_filesystems(&["fs_name"]).unwrap();
        let fs_uuid = fs_results[0].1;
        assert!(match pool.destroy_filesystems(&[fs_uuid, Uuid::new_v4()]) {
            Ok(filesystems) => filesystems == vec![fs_uuid],
            _ => false,
        });
    }

    #[test]
    /// Creating an empty list of filesystems should succeed, always
    fn create_fs_none() {
        let mut engine = SimEngine::new();
        let (uuid, _) = engine.create_pool("pool_name", &[], None, false).unwrap();
        let mut pool = engine.get_pool(&uuid).unwrap();
        assert!(match pool.create_filesystems(&[]) {
            Ok(names) => names.is_empty(),
            _ => false,
        });
    }

    #[test]
    /// Creating a non-empty list of filesystems always succeeds.
    fn create_fs_some() {
        let mut engine = SimEngine::new();
        let (uuid, _) = engine.create_pool("pool_name", &[], None, false).unwrap();
        let mut pool = engine.get_pool(&uuid).unwrap();
        assert!(match pool.create_filesystems(&["name"]) {
            Ok(names) => (names.len() == 1) & (names[0].0 == "name"),
            _ => false,
        });
    }

    #[test]
    /// Creating a an already existing filesystem fails.
    fn create_fs_conflict() {
        let fs_name = "fs_name";
        let mut engine = SimEngine::new();
        let (uuid, _) = engine.create_pool("pool_name", &[], None, false).unwrap();
        let mut pool = engine.get_pool(&uuid).unwrap();
        pool.create_filesystems(&[fs_name]).unwrap();
        assert!(match pool.create_filesystems(&[fs_name]) {
            Err(EngineError::Engine(ErrorEnum::AlreadyExists, _)) => true,
            _ => false,
        });
    }

    #[test]
    /// Requesting identical filesystems succeeds.
    fn create_fs_dups() {
        let fs_name = "fs_name";
        let mut engine = SimEngine::new();
        let (uuid, _) = engine.create_pool("pool_name", &[], None, false).unwrap();
        let mut pool = engine.get_pool(&uuid).unwrap();
        assert!(match pool.create_filesystems(&[fs_name, fs_name]) {
            Ok(names) => (names.len() == 1) & (names[0].0 == fs_name),
            _ => false,
        });
    }

    #[test]
    /// Adding a list of devices to an empty pool should yield list.
    fn add_device_empty() {
        let mut engine = SimEngine::new();
        let (uuid, _) = engine.create_pool("pool_name", &[], None, false).unwrap();
        let mut pool = engine.get_pool(&uuid).unwrap();
        let devices = [Path::new("/s/a"), Path::new("/s/b")];
        assert!(match pool.add_blockdevs(&devices, false) {
            Ok(devs) => devs == devices,
            _ => false,
        });
    }
}

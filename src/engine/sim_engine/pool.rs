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
use engine::Pool;
use engine::RenameAction;

use engine::engine::Redundancy;

use super::super::super::types::Bytes;

use super::blockdev::SimDev;
use super::filesystem::SimFilesystem;
use super::randomization::Randomizer;

use super::super::structures::PoolTableValue;

#[derive(Debug)]
pub struct SimPool {
    name: String,
    pool_uuid: Uuid,
    pub block_devs: BTreeMap<PathBuf, SimDev>,
    pub cache_devs: BTreeMap<PathBuf, SimDev>,
    pub filesystems: BTreeMap<String, SimFilesystem>,
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
            filesystems: BTreeMap::new(),
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

    fn destroy_filesystems<'a, 'b>(&'a mut self,
                                   fs_names: &[&'b str])
                                   -> EngineResult<Vec<&'b str>> {
        destroy_filesystems!{self; fs_names}
    }

    fn destroy(self) -> EngineResult<()> {
        // Nothing to do here.
        Ok(())
    }

    fn create_filesystems<'a, 'b, 'c>(&'a mut self,
                                      specs: &[(&'b str, &'c str, Option<Bytes>)])
                                      -> EngineResult<Vec<&'b str>> {
        let mut temp = Vec::new();
        for spec in specs {
            temp.push(spec);
        }
        temp.sort();
        temp.dedup();

        let names = BTreeSet::from_iter(temp.iter().map(|x| x.0));
        if names.len() < temp.len() {
            let error_message = "duplicate_names in filesystem spec";
            return Err(EngineError::Engine(ErrorEnum::Error, error_message.into()));
        }

        for spec in temp.iter() {
            let name = spec.0;
            if self.filesystems.contains_key(name) {
                return Err(EngineError::Engine(ErrorEnum::AlreadyExists, name.into()));
            }
        }

        let mut names = Vec::new();
        for spec in temp.iter() {
            let (name, mountpoint, quota) = **spec;
            let new_filesystem = SimFilesystem::new(Uuid::new_v4(), mountpoint, quota);
            self.filesystems.insert(name.into(), new_filesystem);
            names.push(name);
        }

        Ok(names)
    }

    fn create_snapshot<'a, 'b, 'c>(&'a mut self,
                                   snapshot_name: &'b str,
                                   source: &'c str)
                                   -> EngineResult<&'b str> {

        let parent_id = try!(self.filesystems
                .get(source)
                .ok_or(EngineError::Engine(ErrorEnum::NotFound, String::from(source))))
            .fs_id;

        let names = try!(self.create_filesystems(&[(snapshot_name, "", None)]));

        let new_snapshot = try!(self.filesystems
            .get_mut(snapshot_name)
            .ok_or(EngineError::Engine(ErrorEnum::NotFound, String::from(snapshot_name))));

        new_snapshot.nearest_ancestor = Some(parent_id);

        Ok(names[0])
    }

    // Should verify that block devices are not required by pool, but does not.
    fn remove_blockdevs(&mut self, paths: &[&Path]) -> EngineResult<Vec<PathBuf>> {
        let devices = BTreeSet::from_iter(paths);

        let mut removed = vec![];
        for dev in devices {
            let pathbuf = dev.to_path_buf();
            match self.block_devs.remove(&pathbuf) {
                Some(_) => removed.push(pathbuf),
                _ => {}
            }
        }
        Ok(removed)
    }

    // Should verify that block devices are not required by pool, but does not.
    fn remove_cachedevs(&mut self, paths: &[&Path]) -> EngineResult<Vec<PathBuf>> {
        let devices = BTreeSet::from_iter(paths);

        let mut removed = vec![];
        for dev in devices {
            let pathbuf = dev.to_path_buf();
            match self.cache_devs.remove(&pathbuf) {
                Some(_) => removed.push(pathbuf),
                _ => {}
            }
        }
        Ok(removed)
    }

    fn rename_filesystem(&mut self, old_name: &str, new_name: &str) -> EngineResult<RenameAction> {
        rename_filesystem!{self; old_name; new_name}
    }

    fn uuid(&self) -> &Uuid {
        &self.pool_uuid
    }

    fn rename(&mut self, name: &str) {
        self.name = name.to_owned();
    }
}

impl PoolTableValue for SimPool {
    fn uuid(&self) -> &Uuid {
        &self.pool_uuid
    }

    fn name(&self) -> &str {
        &self.name
    }
}


#[cfg(test)]
mod tests {

    use std::path::Path;

    use engine::Engine;
    use engine::ErrorEnum;
    use engine::EngineError;
    use engine::RenameAction;

    use super::super::SimEngine;

    #[test]
    /// Renaming a filesystem on an empty pool always works
    fn rename_empty() {
        let pool_name = "name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_pool(pool_name).unwrap();
        assert!(match pool.rename_filesystem("old_name", "new_name") {
            Ok(RenameAction::NoSource) => true,
            _ => false,
        });
    }

    #[test]
    /// Renaming a filesystem to itself on an empty pool always works.
    fn rename_empty_identity() {
        let pool_name = "name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_pool(pool_name).unwrap();
        assert!(match pool.rename_filesystem("old_name", "old_name") {
            Ok(RenameAction::Identity) => true,
            _ => false,
        });
    }

    #[test]
    /// Renaming a filesystem to itself always works
    fn rename_identity() {
        let name = "name";
        let pool_name = "name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_pool(pool_name).unwrap();
        assert!(match pool.rename_filesystem(name, name) {
            Ok(RenameAction::Identity) => true,
            _ => false,
        });
    }

    #[test]
    /// Renaming a filesystem to another filesystem should work if new name not taken
    fn rename_happens() {
        let old_name = "old_name";
        let pool_name = "name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_pool(pool_name).unwrap();
        pool.create_filesystems(&[(old_name, "", None)]).unwrap();
        assert!(match pool.rename_filesystem(old_name, "new_name") {
            Ok(RenameAction::Renamed) => true,
            _ => false,
        });
    }

    #[test]
    /// Renaming a filesystem to another filesystem should fail if new name taken
    fn rename_fails() {
        let old_name = "old_name";
        let new_name = "new_name";
        let pool_name = "name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_pool(pool_name).unwrap();
        pool.create_filesystems(&[(old_name, "", None), (new_name, "", None)]).unwrap();
        assert!(match pool.rename_filesystem(old_name, new_name) {
            Err(EngineError::Engine(ErrorEnum::AlreadyExists, _)) => true,
            _ => false,
        });
    }

    #[test]
    /// Renaming should succeed if old_name absent, new present
    fn rename_no_op() {
        let old_name = "old_name";
        let new_name = "new_name";
        let pool_name = "name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_pool(pool_name).unwrap();
        pool.create_filesystems(&[(new_name, "", None)]).unwrap();
        assert!(match pool.rename_filesystem(old_name, new_name) {
            Ok(RenameAction::NoSource) => true,
            _ => false,
        });
    }

    #[test]
    /// Removing an empty list of filesystems should always succeed
    fn destroy_fs_empty() {
        let name = "name";
        let mut engine = SimEngine::new();
        engine.create_pool(name, &[], None, false).unwrap();
        let mut pool = engine.get_pool(name).unwrap();
        assert!(match pool.destroy_filesystems(&[]) {
            Ok(names) => names.is_empty(),
            _ => false,
        });
    }

    #[test]
    /// Removing a non-empty list of filesystems should succeed on empty pool
    fn destroy_fs_some() {
        let name = "name";
        let mut engine = SimEngine::new();
        engine.create_pool(name, &[], None, false).unwrap();
        let mut pool = engine.get_pool(name).unwrap();
        assert!(pool.destroy_filesystems(&["fs"]).is_ok());
    }

    #[test]
    /// Removing a non-empty list of filesystems should succeed on any pool
    fn destroy_fs_any() {
        let pool_name = "name";
        let fs_name = "fs_name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], None, false).unwrap();
        let mut pool = engine.get_pool(pool_name).unwrap();
        pool.create_filesystems(&[(fs_name, "", None)]).unwrap();
        assert!(match pool.destroy_filesystems(&[fs_name, "other"]) {
            Ok(names) => names == vec![fs_name],
            _ => false,
        });
    }

    #[test]
    /// Creating an empty list of filesystems should succeed, always
    fn create_fs_none() {
        let pool_name = "pool_name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], None, false).unwrap();
        let mut pool = engine.get_pool(pool_name).unwrap();
        assert!(match pool.create_filesystems(&[]) {
            Ok(names) => names.is_empty(),
            _ => false,
        });
    }

    #[test]
    /// Creating a non-empty list of filesystems always succeeds.
    fn create_fs_some() {
        let pool_name = "pool_name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], None, false).unwrap();
        let mut pool = engine.get_pool(pool_name).unwrap();
        assert!(match pool.create_filesystems(&[("name", "", None)]) {
            Ok(names) => (names.len() == 1) & (names[0] == "name"),
            _ => false,
        });
    }

    #[test]
    /// Creating a an already existing filesystem fails.
    fn create_fs_conflict() {
        let pool_name = "pool_name";
        let fs_name = "fs_name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], None, false).unwrap();
        let mut pool = engine.get_pool(pool_name).unwrap();
        pool.create_filesystems(&[(fs_name, "", None)]).unwrap();
        assert!(match pool.create_filesystems(&[(fs_name, "", None)]) {
            Err(EngineError::Engine(ErrorEnum::AlreadyExists, _)) => true,
            _ => false,
        });
    }

    #[test]
    /// Requesting identical filesystems succeeds.
    fn create_fs_dups() {
        let fs_name = "fs_name";
        let pool_name = "pool_name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], None, false).unwrap();
        let mut pool = engine.get_pool(pool_name).unwrap();
        assert!(match pool.create_filesystems(&[(fs_name, "", None), (fs_name, "", None)]) {
            Ok(names) => (names.len() == 1) & (names[0] == fs_name),
            _ => false,
        });
    }

    #[test]
    /// Requesting filesystems with same name but different specs fails.
    fn create_fs_conflicts() {
        let fs_name = "fs_name";
        let pool_name = "pool_name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], None, false).unwrap();
        let mut pool = engine.get_pool(pool_name).unwrap();
        assert!(match pool.create_filesystems(&[(fs_name, "", None), (fs_name, "/", None)]) {
            Err(EngineError::Engine(ErrorEnum::Error, _)) => true,
            _ => false,
        });
    }

    #[test]
    /// Removing an empty list of devices from an empty pool yields empty list.
    fn remove_device_empty() {
        let pool_name = "pool_name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], None, false).unwrap();
        let mut pool = engine.get_pool(pool_name).unwrap();
        assert!(match pool.remove_blockdevs(&[]) {
            Ok(devs) => devs.is_empty(),
            _ => false,
        });
    }

    #[test]
    /// Removing a non-empyt list of devices from empty pool yields empty list.
    fn remove_device_empty_2() {
        let pool_name = "pool_name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], None, false).unwrap();
        let mut pool = engine.get_pool(pool_name).unwrap();
        assert!(match pool.remove_blockdevs(&[Path::new("/s/b")]) {
            Ok(devs) => devs.is_empty(),
            _ => false,
        });
    }

    #[test]
    /// Removing a list of devices from non-empty pool yields intersection.
    fn remove_device_intersection() {
        let pool_name = "pool_name";
        let mut engine = SimEngine::new();
        let devices = [Path::new("/s/a"), Path::new("/s/b")];
        engine.create_pool(pool_name, &devices, None, false).unwrap();
        let mut pool = engine.get_pool(pool_name).unwrap();
        let remove_devices = [Path::new("/s/a"), Path::new("/s/c")];
        assert!(match pool.remove_blockdevs(&remove_devices) {
            Ok(devs) => devs == [Path::new("/s/a")],
            _ => false,
        });
    }

    #[test]
    /// Adding a list of devices to an empty pool should yield list.
    fn add_device_empty() {
        let pool_name = "pool_name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], None, false).unwrap();
        let mut pool = engine.get_pool(pool_name).unwrap();
        let devices = [Path::new("/s/a"), Path::new("/s/b")];
        assert!(match pool.add_blockdevs(&devices, false) {
            Ok(devs) => devs == devices,
            _ => false,
        });
    }
}

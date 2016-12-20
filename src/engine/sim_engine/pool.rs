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

use engine::Dev;
use engine::EngineError;
use engine::EngineResult;
use engine::ErrorEnum;
use engine::Filesystem;
use engine::Pool;
use engine::RenameAction;

use super::blockdev::SimDev;
use super::filesystem::SimFilesystem;
use super::randomization::Randomizer;

use uuid::Uuid;

#[derive(Debug)]
pub struct SimPool {
    pub block_devs: Vec<SimDev>,
    pub cache_devs: Vec<SimDev>,
    pub filesystems: BTreeMap<String, SimFilesystem>,
    pub raid_level: u16,
    rdm: Rc<RefCell<Randomizer>>,
}

impl SimPool {
    pub fn new_pool(rdm: Rc<RefCell<Randomizer>>,
                    blockdevs: &[SimDev],
                    raid_level: u16)
                    -> SimPool {

        let mut vec = Vec::new();
        vec.extend_from_slice(blockdevs);
        let new_pool = SimPool {
            block_devs: vec,
            filesystems: BTreeMap::new(),
            cache_devs: Vec::new(),
            raid_level: raid_level,
            rdm: rdm,
        };

        new_pool
    }
}

impl Pool for SimPool {
    fn add_blockdevs(&mut self, paths: &[&Path], _force: bool) -> EngineResult<Vec<PathBuf>> {
        let rdm = self.rdm.clone();
        let devices = BTreeSet::from_iter(paths);
        self.block_devs.extend(devices.iter().map(|p| SimDev::new(rdm.clone(), p)));
        Ok(devices.iter().map(|d| d.to_path_buf()).collect())
    }

    fn add_cachedevs(&mut self, paths: &[&Path], _force: bool) -> EngineResult<Vec<PathBuf>> {
        let rdm = self.rdm.clone();
        let devices = BTreeSet::from_iter(paths);
        self.cache_devs.extend(devices.iter().map(|p| SimDev::new(rdm.clone(), p)));
        Ok(devices.iter().map(|d| d.to_path_buf()).collect())
    }

    fn destroy_filesystems<'a, 'b>(&'a mut self,
                                   fs_names: &[&'b str])
                                   -> EngineResult<Vec<&'b str>> {
        destroy_filesystems!{self; fs_names}
    }

    fn create_filesystems<'a, 'b, 'c>(&'a mut self,
                                      specs: &[(&'b str, &'c str, Option<u64>)])
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
            return Err(EngineError::Stratis(ErrorEnum::Error(error_message.into())));
        }

        for spec in temp.iter() {
            let name = spec.0;
            if self.filesystems.contains_key(name) {
                return Err(EngineError::Stratis(ErrorEnum::AlreadyExists(name.into())));
            }
        }

        let mut names = Vec::new();
        for spec in temp.iter() {
            let (name, mountpoint, quota) = **spec;
            let new_filesystem = SimFilesystem::new_filesystem(Uuid::new_v4(), mountpoint, quota);
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
                .ok_or(EngineError::Stratis(ErrorEnum::NotFound(String::from(source)))))
            .fs_id;

        let names = try!(self.create_filesystems(&[(snapshot_name, "", None)]));

        let new_snapshot = try!(self.filesystems
            .get_mut(snapshot_name)
            .ok_or(EngineError::Stratis(ErrorEnum::NotFound(String::from(snapshot_name)))));

        new_snapshot.nearest_ancestor = Some(parent_id);

        Ok(names[0])
    }

    fn filesystems(&mut self) -> BTreeMap<&str, &mut Filesystem> {
        BTreeMap::from_iter(self.filesystems
            .iter_mut()
            .map(|x| (x.0 as &str, x.1 as &mut Filesystem)))
    }

    fn blockdevs(&mut self) -> Vec<&mut Dev> {
        Vec::from_iter(self.block_devs.iter_mut().map(|x| x as &mut Dev))
    }

    fn cachedevs(&mut self) -> Vec<&mut Dev> {
        Vec::from_iter(self.cache_devs.iter_mut().map(|x| x as &mut Dev))
    }

    fn remove_blockdev(&mut self, path: &Path) -> EngineResult<()> {
        let index = self.block_devs.iter().position(|x| x.has_same(path));
        match index {
            Some(index) => {
                self.block_devs.remove(index);
                return Ok(());
            }
            None => {
                return Err(EngineError::Stratis(ErrorEnum::NotFound(String::from(path.to_str()
                    .unwrap()))))
            }
        }
        Ok(())
    }

    fn remove_cachedev(&mut self, path: &Path) -> EngineResult<()> {
        let index = self.cache_devs.iter().position(|x| x.has_same(path));

        match index {
            Some(index) => {
                self.cache_devs.remove(index);
                return Ok(());
            }
            None => {
                return Err(EngineError::Stratis(ErrorEnum::NotFound(String::from(path.to_str()
                    .unwrap()))))
            }
        }
        Ok(())
    }

    fn rename_filesystem(&mut self, old_name: &str, new_name: &str) -> EngineResult<RenameAction> {
        rename_filesystem!{self; old_name; new_name}
    }
}


#[cfg(test)]
mod tests {

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
        engine.create_pool(pool_name, &[], 0, false).unwrap();
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
        engine.create_pool(pool_name, &[], 0, false).unwrap();
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
        engine.create_pool(pool_name, &[], 0, false).unwrap();
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
        engine.create_pool(pool_name, &[], 0, false).unwrap();
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
        engine.create_pool(pool_name, &[], 0, false).unwrap();
        let pool = engine.get_pool(pool_name).unwrap();
        pool.create_filesystems(&[(old_name, "", None), (new_name, "", None)]).unwrap();
        assert!(match pool.rename_filesystem(old_name, new_name) {
            Err(EngineError::Stratis(ErrorEnum::AlreadyExists(_))) => true,
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
        engine.create_pool(pool_name, &[], 0, false).unwrap();
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
        engine.create_pool(name, &[], 0, false).unwrap();
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
        engine.create_pool(name, &[], 0, false).unwrap();
        let mut pool = engine.get_pool(name).unwrap();
        assert!(pool.destroy_filesystems(&["fs"]).is_ok());
    }

    #[test]
    /// Removing a non-empty list of filesystems should succeed on any pool
    fn destroy_fs_any() {
        let pool_name = "name";
        let fs_name = "fs_name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], 0, false).unwrap();
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
        engine.create_pool(pool_name, &[], 0, false).unwrap();
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
        engine.create_pool(pool_name, &[], 0, false).unwrap();
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
        engine.create_pool(pool_name, &[], 0, false).unwrap();
        let mut pool = engine.get_pool(pool_name).unwrap();
        pool.create_filesystems(&[(fs_name, "", None)]).unwrap();
        assert!(match pool.create_filesystems(&[(fs_name, "", None)]) {
            Err(EngineError::Stratis(ErrorEnum::AlreadyExists(_))) => true,
            _ => false,
        });
    }

    #[test]
    /// Requesting identical filesystems succeeds.
    fn create_fs_dups() {
        let fs_name = "fs_name";
        let pool_name = "pool_name";
        let mut engine = SimEngine::new();
        engine.create_pool(pool_name, &[], 0, false).unwrap();
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
        engine.create_pool(pool_name, &[], 0, false).unwrap();
        let mut pool = engine.get_pool(pool_name).unwrap();
        assert!(match pool.create_filesystems(&[(fs_name, "", None), (fs_name, "/", None)]) {
            Err(EngineError::Stratis(ErrorEnum::Error(_))) => true,
            _ => false,
        });
    }
}

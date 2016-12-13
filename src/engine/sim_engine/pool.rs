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

use engine::Cache;
use engine::Dev;
use engine::EngineError;
use engine::EngineResult;
use engine::ErrorEnum;
use engine::Filesystem;
use engine::Pool;
use engine::RenameAction;

use super::blockdev::SimDev;
use super::cache::SimCacheDev;
use super::filesystem::SimFilesystem;
use super::randomization::Randomizer;

use uuid::Uuid;

#[derive(Debug)]
pub struct SimPool {
    pub block_devs: Vec<SimDev>,
    pub cache_devs: Vec<SimCacheDev>,
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
        self.block_devs.extend(devices.iter().map(|p| SimDev::new_dev(rdm.clone(), p)));
        Ok(devices.iter().map(|d| d.to_path_buf()).collect())
    }

    fn add_cachedevs(&mut self, paths: &[&Path], _force: bool) -> EngineResult<Vec<PathBuf>> {
        let rdm = self.rdm.clone();
        let devices = BTreeSet::from_iter(paths);
        self.cache_devs.extend(devices.iter().map(|p| SimCacheDev::new_cache(rdm.clone(), p)));
        Ok(devices.iter().map(|d| d.to_path_buf()).collect())
    }

    fn destroy_filesystems<'a, 'b>(&'a mut self,
                                   fs_names: &[&'b str])
                                   -> EngineResult<Vec<&'b str>> {
        destroy_filesystems!{self; fs_names}
    }

    fn create_filesystem(&mut self,
                         name: &str,
                         mount_point: &str,
                         quota_size: Option<u64>)
                         -> EngineResult<Uuid> {

        match self.filesystems.get(name) {
            Some(_) => {
                return Err(EngineError::Stratis(ErrorEnum::AlreadyExists(String::from(name))));
            }
            None => {}
        }

        let fs_uuid = Uuid::new_v4();
        let new_filesystem = SimFilesystem::new_filesystem(fs_uuid, mount_point, quota_size);
        self.filesystems.insert(name.into(), new_filesystem);
        Ok(fs_uuid)
    }

    fn create_snapshot(&mut self, snapshot_name: &str, source: &str) -> EngineResult<Uuid> {

        let parent_id = try!(self.filesystems
                .get(source)
                .ok_or(EngineError::Stratis(ErrorEnum::NotFound(String::from(source)))))
            .fs_id;

        let uuid = try!(self.create_filesystem(&snapshot_name, &String::from(""), None));

        let new_snapshot = try!(self.filesystems
            .get_mut(snapshot_name)
            .ok_or(EngineError::Stratis(ErrorEnum::NotFound(String::from(snapshot_name)))));

        new_snapshot.nearest_ancestor = Some(parent_id);

        Ok(uuid)
    }

    fn filesystems(&mut self) -> BTreeMap<&str, &mut Filesystem> {
        BTreeMap::from_iter(self.filesystems
            .iter_mut()
            .map(|x| (x.0 as &str, x.1 as &mut Filesystem)))
    }

    fn blockdevs(&mut self) -> Vec<&mut Dev> {
        Vec::from_iter(self.block_devs.iter_mut().map(|x| x as &mut Dev))
    }

    fn cachedevs(&mut self) -> Vec<&mut Cache> {
        Vec::from_iter(self.cache_devs.iter_mut().map(|x| x as &mut Cache))
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
        engine.create_pool(pool_name, &vec![], 0, false).unwrap();
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
        engine.create_pool(pool_name, &vec![], 0, false).unwrap();
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
        engine.create_pool(pool_name, &vec![], 0, false).unwrap();
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
        engine.create_pool(pool_name, &vec![], 0, false).unwrap();
        let pool = engine.get_pool(pool_name).unwrap();
        pool.create_filesystem(old_name, "", None).unwrap();
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
        engine.create_pool(pool_name, &vec![], 0, false).unwrap();
        let pool = engine.get_pool(pool_name).unwrap();
        pool.create_filesystem(new_name, "", None).unwrap();
        pool.create_filesystem(old_name, "", None).unwrap();
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
        engine.create_pool(pool_name, &vec![], 0, false).unwrap();
        let pool = engine.get_pool(pool_name).unwrap();
        pool.create_filesystem(new_name, "", None).unwrap();
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
        engine.create_pool(name, &vec![], 0, false).unwrap();
        let mut pool = engine.get_pool(name).unwrap();
        assert!(match pool.destroy_filesystems(&vec![]) {
            Ok(names) => names.is_empty(),
            _ => false,
        });
    }

    #[test]
    /// Removing a non-empty list of filesystems should succeed on empty pool
    fn destroy_fs_some() {
        let name = "name";
        let mut engine = SimEngine::new();
        engine.create_pool(name, &vec![], 0, false).unwrap();
        let mut pool = engine.get_pool(name).unwrap();
        assert!(pool.destroy_filesystems(&vec!["fs"]).is_ok());
    }
}

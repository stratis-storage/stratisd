// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::collections::hash_map::RandomState;
use std::iter::FromIterator;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::vec::Vec;

use uuid::Uuid;

use devicemapper::Sectors;

use super::super::consts::IEC;
use super::super::engine::{Filesystem, HasName, HasUuid, Pool};
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::structures::Table;
use super::super::types::{FilesystemUuid, PoolUuid, RenameAction, Redundancy};

use super::blockdev::SimDev;
use super::filesystem::SimFilesystem;
use super::randomization::Randomizer;

#[derive(Debug)]
pub struct SimPool {
    name: String,
    pool_uuid: PoolUuid,
    pub block_devs: HashMap<PathBuf, SimDev>,
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

        let devices: HashSet<_, RandomState> = HashSet::from_iter(paths);
        let device_pairs = devices
            .iter()
            .map(|p| (p.to_path_buf(), SimDev::new(rdm.clone(), p)));
        SimPool {
            name: name.to_owned(),
            pool_uuid: Uuid::new_v4(),
            block_devs: HashMap::from_iter(device_pairs),
            filesystems: Table::default(),
            redundancy: redundancy,
            rdm: rdm.clone(),
        }
    }

    pub fn check(&mut self) -> () {}

    pub fn has_filesystems(&self) -> bool {
        !self.filesystems.is_empty()
    }
}

impl Pool for SimPool {
    fn add_blockdevs(&mut self, paths: &[&Path], _force: bool) -> EngineResult<Vec<PathBuf>> {
        let rdm = self.rdm.clone();
        let devices: HashSet<_, RandomState> = HashSet::from_iter(paths);
        let device_pairs = devices
            .iter()
            .map(|p| (p.to_path_buf(), SimDev::new(rdm.clone(), p)));
        self.block_devs.extend(device_pairs);
        Ok(devices.iter().map(|d| d.to_path_buf()).collect())
    }

    fn destroy_filesystems<'a, 'b>(&'a mut self,
                                   fs_uuids: &[&'b FilesystemUuid])
                                   -> EngineResult<Vec<&'b FilesystemUuid>> {
        let mut removed = Vec::new();
        for uuid in fs_uuids {
            if self.filesystems.remove_by_uuid(uuid).is_some() {
                removed.push(*uuid);
            }
        }
        Ok(removed)
    }

    fn destroy(self) -> EngineResult<()> {
        // Nothing to do here.
        Ok(())
    }

    fn create_filesystems<'a, 'b>(&'a mut self,
                                  specs: &[&'b str])
                                  -> EngineResult<Vec<(&'b str, FilesystemUuid)>> {
        let names: HashSet<_, RandomState> = HashSet::from_iter(specs);
        for name in &names {
            if self.filesystems.contains_name(name) {
                return Err(EngineError::Engine(ErrorEnum::AlreadyExists, name.to_string()));
            }
        }

        let mut result = Vec::new();
        for name in &names {
            let uuid = Uuid::new_v4();
            let new_filesystem = SimFilesystem::new(uuid, name);
            self.filesystems.insert(new_filesystem);
            result.push((**name, uuid));
        }

        Ok(result)
    }

    fn rename_filesystem(&mut self,
                         uuid: &FilesystemUuid,
                         new_name: &str)
                         -> EngineResult<RenameAction> {
        rename_filesystem_pre!(self; uuid; new_name);

        let mut filesystem =
            self.filesystems
                .remove_by_uuid(uuid)
                .expect("Must succeed since self.filesystems.get_by_uuid() returned a value");

        filesystem.rename(new_name);
        self.filesystems.insert(filesystem);
        Ok(RenameAction::Renamed)
    }

    fn rename(&mut self, name: &str) {
        self.name = name.to_owned();
    }

    fn get_filesystem(&mut self, uuid: &FilesystemUuid) -> Option<&mut Filesystem> {
        get_filesystem!(self; uuid)
    }

    fn total_physical_size(&self) -> Sectors {
        // We choose to make our pools very big, and we can change that
        // if it is inconvenient.
        Sectors(IEC::Ei)
    }

    fn total_physical_used(&self) -> EngineResult<Sectors> {
        Ok(Sectors(0))
    }
}

impl HasUuid for SimPool {
    fn uuid(&self) -> &PoolUuid {
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
        let mut engine = SimEngine::default();
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
        let mut engine = SimEngine::default();
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
        let mut engine = SimEngine::default();
        let (uuid, _) = engine.create_pool("name", &[], None, false).unwrap();
        let pool = engine.get_pool(&uuid).unwrap();
        let results = pool.create_filesystems(&[old_name, new_name]).unwrap();
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
        let mut engine = SimEngine::default();
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
        let mut engine = SimEngine::default();
        let (uuid, _) = engine.create_pool("name", &[], None, false).unwrap();
        let pool = engine.get_pool(&uuid).unwrap();
        assert!(match pool.destroy_filesystems(&[]) {
                    Ok(names) => names.is_empty(),
                    _ => false,
                });
    }

    #[test]
    /// Removing a non-empty list of filesystems should succeed on empty pool
    fn destroy_fs_some() {
        let mut engine = SimEngine::default();
        let (uuid, _) = engine.create_pool("name", &[], None, false).unwrap();
        let pool = engine.get_pool(&uuid).unwrap();
        assert!(pool.destroy_filesystems(&[&Uuid::new_v4()]).is_ok());
    }

    #[test]
    /// Removing a non-empty list of filesystems should succeed on any pool
    fn destroy_fs_any() {
        let mut engine = SimEngine::default();
        let (uuid, _) = engine.create_pool("name", &[], None, false).unwrap();
        let pool = engine.get_pool(&uuid).unwrap();
        let fs_results = pool.create_filesystems(&["fs_name"]).unwrap();
        let fs_uuid = fs_results[0].1;
        assert!(match pool.destroy_filesystems(&[&fs_uuid, &Uuid::new_v4()]) {
                    Ok(filesystems) => filesystems == vec![&fs_uuid],
                    _ => false,
                });
    }

    #[test]
    /// Creating an empty list of filesystems should succeed, always
    fn create_fs_none() {
        let mut engine = SimEngine::default();
        let (uuid, _) = engine
            .create_pool("pool_name", &[], None, false)
            .unwrap();
        let pool = engine.get_pool(&uuid).unwrap();
        assert!(match pool.create_filesystems(&[]) {
                    Ok(names) => names.is_empty(),
                    _ => false,
                });
    }

    #[test]
    /// Creating a non-empty list of filesystems always succeeds.
    fn create_fs_some() {
        let mut engine = SimEngine::default();
        let (uuid, _) = engine
            .create_pool("pool_name", &[], None, false)
            .unwrap();
        let pool = engine.get_pool(&uuid).unwrap();
        assert!(match pool.create_filesystems(&["name"]) {
                    Ok(names) => (names.len() == 1) & (names[0].0 == "name"),
                    _ => false,
                });
    }

    #[test]
    /// Creating a an already existing filesystem fails.
    fn create_fs_conflict() {
        let fs_name = "fs_name";
        let mut engine = SimEngine::default();
        let (uuid, _) = engine
            .create_pool("pool_name", &[], None, false)
            .unwrap();
        let pool = engine.get_pool(&uuid).unwrap();
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
        let mut engine = SimEngine::default();
        let (uuid, _) = engine
            .create_pool("pool_name", &[], None, false)
            .unwrap();
        let pool = engine.get_pool(&uuid).unwrap();
        assert!(match pool.create_filesystems(&[fs_name, fs_name]) {
                    Ok(names) => (names.len() == 1) & (names[0].0 == fs_name),
                    _ => false,
                });
    }

    #[test]
    /// Adding a list of devices to an empty pool should yield list.
    fn add_device_empty() {
        let mut engine = SimEngine::default();
        let (uuid, _) = engine
            .create_pool("pool_name", &[], None, false)
            .unwrap();
        let pool = engine.get_pool(&uuid).unwrap();
        let devices = [Path::new("/s/a"), Path::new("/s/b")];
        assert!(match pool.add_blockdevs(&devices, false) {
                    Ok(devs) => devs.len() == devices.len(),
                    _ => false,
                });
    }
}

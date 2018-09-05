// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cell::RefCell;
use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;
use std::path::Path;
use std::rc::Rc;
use std::vec::Vec;

use uuid::Uuid;

use devicemapper::{Sectors, IEC};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::engine::{BlockDev, Filesystem, Pool};
use super::super::structures::Table;
use super::super::types::{
    BlockDevTier, DevUuid, FilesystemUuid, MaybeDbusPath, Name, PoolUuid, Redundancy, RenameAction,
};

use super::blockdev::SimDev;
use super::filesystem::SimFilesystem;
use super::randomization::Randomizer;

#[derive(Debug)]
pub struct SimPool {
    block_devs: HashMap<DevUuid, SimDev>,
    cache_devs: HashMap<DevUuid, SimDev>,
    filesystems: Table<SimFilesystem>,
    redundancy: Redundancy,
    rdm: Rc<RefCell<Randomizer>>,
    dbus_path: MaybeDbusPath,
}

impl SimPool {
    pub fn new(
        rdm: &Rc<RefCell<Randomizer>>,
        paths: &[&Path],
        redundancy: Redundancy,
    ) -> (PoolUuid, SimPool) {
        let devices: HashSet<_, RandomState> = HashSet::from_iter(paths);
        let device_pairs = devices.iter().map(|p| SimDev::new(Rc::clone(rdm), p));
        (
            Uuid::new_v4(),
            SimPool {
                block_devs: HashMap::from_iter(device_pairs),
                cache_devs: HashMap::new(),
                filesystems: Table::default(),
                redundancy,
                rdm: Rc::clone(rdm),
                dbus_path: MaybeDbusPath(None),
            },
        )
    }

    pub fn has_filesystems(&self) -> bool {
        !self.filesystems.is_empty()
    }

    fn get_mut_blockdev_internal(&mut self, uuid: DevUuid) -> Option<(BlockDevTier, &mut SimDev)> {
        let cache_devs = &mut self.cache_devs;
        self.block_devs
            .get_mut(&uuid)
            .and_then(|bd| Some((BlockDevTier::Data, bd)))
            .or_else(move || {
                cache_devs
                    .get_mut(&uuid)
                    .and_then(|bd| Some((BlockDevTier::Cache, bd)))
            })
    }
}

impl Pool for SimPool {
    fn create_filesystems<'a, 'b>(
        &'a mut self,
        _pool_uuid: PoolUuid,
        _pool_name: &str,
        specs: &[(&'b str, Option<Sectors>)],
    ) -> StratisResult<Vec<(&'b str, FilesystemUuid)>> {
        let names: HashMap<_, _> = HashMap::from_iter(specs.iter().map(|&tup| (tup.0, tup.1)));
        for name in names.keys() {
            if self.filesystems.contains_name(name) {
                return Err(StratisError::Engine(
                    ErrorEnum::AlreadyExists,
                    name.to_string(),
                ));
            }
        }

        let mut result = Vec::new();
        for name in names.keys() {
            let uuid = Uuid::new_v4();
            let new_filesystem = SimFilesystem::new();
            self.filesystems
                .insert(Name::new((&**name).to_owned()), uuid, new_filesystem);
            result.push((*name, uuid));
        }

        Ok(result)
    }

    fn add_blockdevs(
        &mut self,
        _pool_uuid: PoolUuid,
        _pool_name: &str,
        paths: &[&Path],
        tier: BlockDevTier,
        _force: bool,
    ) -> StratisResult<Vec<DevUuid>> {
        let devices: HashSet<_, RandomState> = HashSet::from_iter(paths);
        let device_pairs: Vec<_> = devices
            .iter()
            .map(|p| SimDev::new(Rc::clone(&self.rdm), p))
            .collect();
        let ret_uuids = device_pairs.iter().map(|&(uuid, _)| uuid).collect();

        let the_vec = match tier {
            BlockDevTier::Cache => &mut self.cache_devs,
            BlockDevTier::Data => &mut self.block_devs,
        };

        the_vec.extend(device_pairs);
        Ok(ret_uuids)
    }

    fn destroy(&mut self) -> StratisResult<()> {
        // Nothing to do here.
        Ok(())
    }

    fn destroy_filesystems<'a>(
        &'a mut self,
        _pool_name: &str,
        fs_uuids: &[FilesystemUuid],
    ) -> StratisResult<Vec<FilesystemUuid>> {
        let mut removed = Vec::new();
        for &uuid in fs_uuids {
            if self.filesystems.remove_by_uuid(uuid).is_some() {
                removed.push(uuid);
            }
        }
        Ok(removed)
    }

    fn rename_filesystem(
        &mut self,
        _pool_name: &str,
        uuid: FilesystemUuid,
        new_name: &str,
    ) -> StratisResult<RenameAction> {
        rename_filesystem_pre!(self; uuid; new_name);

        let (_, filesystem) = self.filesystems
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.filesystems.get_by_uuid() returned a value");

        self.filesystems
            .insert(Name::new(new_name.to_owned()), uuid, filesystem);

        Ok(RenameAction::Renamed)
    }

    fn snapshot_filesystem(
        &mut self,
        _pool_uuid: PoolUuid,
        _pool_name: &str,
        origin_uuid: FilesystemUuid,
        snapshot_name: &str,
    ) -> StratisResult<(FilesystemUuid, &mut Filesystem)> {
        let uuid = Uuid::new_v4();
        let snapshot = match self.get_filesystem(origin_uuid) {
            Some(_filesystem) => SimFilesystem::new(),
            None => {
                return Err(StratisError::Engine(
                    ErrorEnum::NotFound,
                    origin_uuid.to_string(),
                ));
            }
        };
        self.filesystems
            .insert(Name::new(snapshot_name.to_owned()), uuid, snapshot);
        Ok((
            uuid,
            self.filesystems
                .get_mut_by_uuid(uuid)
                .expect("just inserted")
                .1,
        ))
    }

    fn total_physical_size(&self) -> Sectors {
        // We choose to make our pools very big, and we can change that
        // if it is inconvenient.
        Sectors(IEC::Ei)
    }

    fn total_physical_used(&self) -> StratisResult<Sectors> {
        Ok(Sectors(0))
    }

    fn filesystems(&self) -> Vec<(Name, FilesystemUuid, &Filesystem)> {
        self.filesystems
            .iter()
            .map(|(name, uuid, x)| (name.clone(), *uuid, x as &Filesystem))
            .collect()
    }

    fn filesystems_mut(&mut self) -> Vec<(Name, FilesystemUuid, &mut Filesystem)> {
        self.filesystems
            .iter_mut()
            .map(|(name, uuid, x)| (name.clone(), *uuid, x as &mut Filesystem))
            .collect()
    }

    fn get_filesystem(&self, uuid: FilesystemUuid) -> Option<(Name, &Filesystem)> {
        self.filesystems
            .get_by_uuid(uuid)
            .map(|(name, p)| (name, p as &Filesystem))
    }

    fn get_mut_filesystem(&mut self, uuid: FilesystemUuid) -> Option<(Name, &mut Filesystem)> {
        self.filesystems
            .get_mut_by_uuid(uuid)
            .map(|(name, p)| (name, p as &mut Filesystem))
    }

    fn blockdevs(&self) -> Vec<(DevUuid, &BlockDev)> {
        self.block_devs
            .iter()
            .chain(self.cache_devs.iter())
            .map(|(uuid, bd)| (*uuid, bd as &BlockDev))
            .collect()
    }

    fn blockdevs_mut(&mut self) -> Vec<(DevUuid, &mut BlockDev)> {
        self.block_devs
            .iter_mut()
            .chain(self.cache_devs.iter_mut().into_iter())
            .map(|(uuid, b)| (uuid.clone(), b as &mut BlockDev))
            .collect()
    }

    fn get_blockdev(&self, uuid: DevUuid) -> Option<(BlockDevTier, &BlockDev)> {
        self.block_devs
            .get(&uuid)
            .and_then(|bd| Some((BlockDevTier::Data, bd as &BlockDev)))
            .or_else(move || {
                self.cache_devs
                    .get(&uuid)
                    .and_then(|bd| Some((BlockDevTier::Cache, bd as &BlockDev)))
            })
    }

    fn get_mut_blockdev(&mut self, uuid: DevUuid) -> Option<(BlockDevTier, &mut BlockDev)> {
        self.get_mut_blockdev_internal(uuid)
            .map(|(tier, bd)| (tier, bd as &mut BlockDev))
    }

    fn set_blockdev_user_info(
        &mut self,
        _pool_name: &str,
        uuid: DevUuid,
        user_info: Option<&str>,
    ) -> StratisResult<bool> {
        self.get_mut_blockdev_internal(uuid).map_or_else(
            || {
                Err(StratisError::Engine(
                    ErrorEnum::NotFound,
                    format!("No blockdev for uuid {} found", uuid),
                ))
            },
            |(_, b)| Ok(b.set_user_info(user_info)),
        )
    }

    fn set_dbus_path(&mut self, path: MaybeDbusPath) -> () {
        self.dbus_path = path
    }

    fn get_dbus_path(&self) -> &MaybeDbusPath {
        &self.dbus_path
    }
}

#[cfg(test)]
mod tests {

    use std::path::Path;

    use uuid::Uuid;

    use engine::Engine;

    use super::super::SimEngine;

    use super::*;

    #[test]
    /// Renaming a filesystem on an empty pool always works
    fn rename_empty() {
        let mut engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_mut_pool(uuid).unwrap().1;
        assert!(
            match pool.rename_filesystem(pool_name, Uuid::new_v4(), "new_name") {
                Ok(RenameAction::NoSource) => true,
                _ => false,
            }
        );
    }

    #[test]
    /// Renaming a filesystem to another filesystem should work if new name not taken
    fn rename_happens() {
        let mut engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_mut_pool(uuid).unwrap().1;
        let infos = pool.create_filesystems(uuid, pool_name, &[("old_name", None)])
            .unwrap();
        assert!(
            match pool.rename_filesystem(pool_name, infos[0].1, "new_name") {
                Ok(RenameAction::Renamed) => true,
                _ => false,
            }
        );
    }

    #[test]
    /// Renaming a filesystem to another filesystem should fail if new name taken
    fn rename_fails() {
        let old_name = "old_name";
        let new_name = "new_name";
        let mut engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_mut_pool(uuid).unwrap().1;
        let results =
            pool.create_filesystems(uuid, pool_name, &[(old_name, None), (new_name, None)])
                .unwrap();
        let old_uuid = results.iter().find(|x| x.0 == old_name).unwrap().1;
        assert!(
            match pool.rename_filesystem(pool_name, old_uuid, new_name) {
                Err(StratisError::Engine(ErrorEnum::AlreadyExists, _)) => true,
                _ => false,
            }
        );
    }

    #[test]
    /// Renaming should succeed if old_name absent, new present
    fn rename_no_op() {
        let new_name = "new_name";
        let mut engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_mut_pool(uuid).unwrap().1;
        assert!(
            match pool.rename_filesystem(pool_name, Uuid::new_v4(), new_name) {
                Ok(RenameAction::NoSource) => true,
                _ => false,
            }
        );
    }

    #[test]
    /// Removing an empty list of filesystems should always succeed
    fn destroy_fs_empty() {
        let mut engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_mut_pool(uuid).unwrap().1;
        assert!(match pool.destroy_filesystems(pool_name, &[]) {
            Ok(names) => names.is_empty(),
            _ => false,
        });
    }

    #[test]
    /// Removing a non-empty list of filesystems should succeed on empty pool
    fn destroy_fs_some() {
        let mut engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_mut_pool(uuid).unwrap().1;
        assert!(
            pool.destroy_filesystems(pool_name, &[Uuid::new_v4()])
                .is_ok()
        );
    }

    #[test]
    /// Removing a non-empty list of filesystems should succeed on any pool
    fn destroy_fs_any() {
        let mut engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_mut_pool(uuid).unwrap().1;
        let fs_results = pool.create_filesystems(uuid, pool_name, &[("fs_name", None)])
            .unwrap();
        let fs_uuid = fs_results[0].1;
        assert!(
            match pool.destroy_filesystems(pool_name, &[fs_uuid, Uuid::new_v4()]) {
                Ok(filesystems) => filesystems == vec![fs_uuid],
                _ => false,
            }
        );
    }

    #[test]
    /// Creating an empty list of filesystems should succeed, always
    fn create_fs_none() {
        let mut engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_mut_pool(uuid).unwrap().1;
        assert!(match pool.create_filesystems(uuid, pool_name, &[]) {
            Ok(names) => names.is_empty(),
            _ => false,
        });
    }

    #[test]
    /// Creating a non-empty list of filesystems always succeeds.
    fn create_fs_some() {
        let mut engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_mut_pool(uuid).unwrap().1;
        assert!(
            match pool.create_filesystems(uuid, pool_name, &[("name", None)]) {
                Ok(names) => (names.len() == 1) & (names[0].0 == "name"),
                _ => false,
            }
        );
    }

    #[test]
    /// Creating a an already existing filesystem fails.
    fn create_fs_conflict() {
        let fs_name = "fs_name";
        let mut engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_mut_pool(uuid).unwrap().1;
        pool.create_filesystems(uuid, pool_name, &[(fs_name, None)])
            .unwrap();
        assert!(
            match pool.create_filesystems(uuid, pool_name, &[(fs_name, None)]) {
                Err(StratisError::Engine(ErrorEnum::AlreadyExists, _)) => true,
                _ => false,
            }
        );
    }

    #[test]
    /// Requesting identical filesystems succeeds.
    fn create_fs_dups() {
        let fs_name = "fs_name";
        let mut engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = engine.create_pool(pool_name, &[], None, false).unwrap();
        let pool = engine.get_mut_pool(uuid).unwrap().1;
        assert!(
            match pool.create_filesystems(uuid, pool_name, &[(fs_name, None), (fs_name, None)]) {
                Ok(names) => (names.len() == 1) & (names[0].0 == fs_name),
                _ => false,
            }
        );
    }

    #[test]
    /// Adding a list of devices to an empty pool should yield list.
    fn add_device_empty() {
        let mut engine = SimEngine::default();
        let uuid = engine.create_pool("pool_name", &[], None, false).unwrap();
        let (pool_name, pool) = engine.get_mut_pool(uuid).unwrap();
        let devices = [Path::new("/s/a"), Path::new("/s/b")];
        assert!(
            match pool.add_blockdevs(uuid, &*pool_name, &devices, BlockDevTier::Data, false) {
                Ok(devs) => devs.len() == devices.len(),
                _ => false,
            }
        );
    }
}

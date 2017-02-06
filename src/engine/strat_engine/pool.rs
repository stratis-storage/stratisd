// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::vec::Vec;
use std::fs::OpenOptions;

use time;
use uuid::Uuid;
use serde_json;

use engine::Dev;
use engine::EngineError;
use engine::EngineResult;
use engine::ErrorEnum;
use engine::Filesystem;
use engine::Pool;
use engine::RenameAction;
use engine::engine::Redundancy;

use super::super::super::types::Bytes;

use super::serde_structs::StratSave;
use super::blockdev::{BlockDev, initialize, resolve_devices};
use super::filesystem::StratFilesystem;
use super::metadata::MIN_MDA_SECTORS;

#[derive(Debug)]
pub struct StratPool {
    pub name: String,
    pub pool_uuid: Uuid,
    pub cache_devs: BTreeMap<PathBuf, BlockDev>,
    pub block_devs: BTreeMap<PathBuf, BlockDev>,
    pub filesystems: BTreeMap<String, StratFilesystem>,
    redundancy: Redundancy,
}

impl StratPool {
    pub fn new(name: &str,
               paths: &[&Path],
               redundancy: Redundancy,
               force: bool)
               -> EngineResult<StratPool> {
        let devices = try!(resolve_devices(paths));
        let pool_uuid = Uuid::new_v4();
        let bds = try!(initialize(&pool_uuid, devices, MIN_MDA_SECTORS, force));

        let mut pool = StratPool {
            name: name.to_owned(),
            pool_uuid: pool_uuid,
            cache_devs: BTreeMap::new(),
            block_devs: bds,
            filesystems: BTreeMap::new(),
            redundancy: redundancy,
        };

        try!(pool.write_metadata());

        Ok(pool)
    }

    /// Read the latest data across all blockdevs
    pub fn read_metadata(&self) -> Option<Vec<u8>> {

        let mut bds: Vec<&BlockDev> = self.block_devs
            .iter()
            .map(|(_, bd)| bd)
            .filter(|bd| bd.bda.last_update_time().is_some())
            .collect();

        bds.sort_by_key(|k| k.bda.last_update_time().unwrap());

        // Only try to read blockdevs with the latest metadata
        let last_update_time = match bds.last() {
            Some(bd) => bd.bda.last_update_time().unwrap(),
            None => return None,
        };

        for bd in bds.iter()
            .rev()
            .take_while(|bd| bd.bda.last_update_time().unwrap() == last_update_time) {
            let mut f = match OpenOptions::new()
                .read(true)
                .open(&bd.devnode) {
                Ok(f) => f,
                Err(_) => continue,
            };
            match bd.bda.load_state(&mut f) {
                Ok(Some(data)) => return Some(data),
                _ => continue,
            }
        }
        None
    }

    /// Write the given data to all blockdevs
    pub fn write_metadata(&mut self) -> EngineResult<()> {

        let data = try!(serde_json::to_string(&self.to_save()));

        // TODO: Cap # of blockdevs written to, as described in SWDD
        // TODO: Check current time against global last updated, and use
        // alternate time value if earlier, as described in SWDD

        let time = time::now().to_timespec();

        for (_, bd) in &mut self.block_devs {
            bd.save_state(&time, data.as_bytes()).unwrap(); // ignoring failure
        }

        Ok(())
    }

    pub fn to_save(&self) -> StratSave {
        StratSave {
            name: self.name.clone(),
            id: self.pool_uuid.simple().to_string(),
            block_devs: self.block_devs
                .iter()
                .map(|(_, bd)| (bd.bda.header.dev_uuid.simple().to_string(), bd.to_save()))
                .collect(),
        }
    }
}

impl Pool for StratPool {
    fn create_filesystems<'a, 'b, 'c>(&'a mut self,
                                      _specs: &[(&'b str, &'c str, Option<Bytes>)])
                                      -> EngineResult<Vec<&'b str>> {
        Ok(vec![])
    }

    fn create_snapshot<'a, 'b, 'c>(&'a mut self,
                                   _snapshot_name: &'b str,
                                   _source: &'c str)
                                   -> EngineResult<&'b str> {
        unimplemented!()
    }

    fn add_blockdevs(&mut self, paths: &[&Path], force: bool) -> EngineResult<Vec<PathBuf>> {
        let devices = try!(resolve_devices(paths));
        let mut bds = try!(initialize(&self.pool_uuid, devices, MIN_MDA_SECTORS, force));
        let bdev_paths = bds.iter().map(|p| p.1.devnode.clone()).collect();
        self.block_devs.append(&mut bds);
        try!(self.write_metadata());
        Ok(bdev_paths)
    }

    fn add_cachedevs(&mut self, paths: &[&Path], force: bool) -> EngineResult<Vec<PathBuf>> {
        let devices = try!(resolve_devices(paths));
        let mut bds = try!(initialize(&self.pool_uuid, devices, MIN_MDA_SECTORS, force));
        let bdev_paths = bds.iter().map(|p| p.1.devnode.clone()).collect();
        self.cache_devs.append(&mut bds);
        Ok(bdev_paths)
    }

    fn destroy_filesystems<'a, 'b>(&'a mut self,
                                   fs_names: &[&'b str])
                                   -> EngineResult<Vec<&'b str>> {
        destroy_filesystems!{self; fs_names}
    }

    fn filesystems(&mut self) -> BTreeMap<&str, &mut Filesystem> {
        unimplemented!()
    }

    fn remove_blockdevs(&mut self, _paths: &[&Path]) -> EngineResult<Vec<PathBuf>> {
        unimplemented!()
    }

    fn remove_cachedevs(&mut self, _paths: &[&Path]) -> EngineResult<Vec<PathBuf>> {
        unimplemented!()
    }

    fn blockdevs(&mut self) -> Vec<&mut Dev> {
        unimplemented!()
    }

    fn cachedevs(&mut self) -> Vec<&mut Dev> {
        unimplemented!()
    }

    fn rename_filesystem(&mut self, old_name: &str, new_name: &str) -> EngineResult<RenameAction> {
        rename_filesystem!{self; old_name; new_name}
    }
}

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

use engine::EngineError;
use engine::EngineResult;
use engine::ErrorEnum;
use engine::Filesystem;
use engine::Pool;
use engine::RenameAction;
use engine::engine::Redundancy;

use super::super::engine::{HasName, HasUuid};
use super::super::structures::Table;

use super::serde_structs::StratSave;
use super::blockdev::{BlockDev, initialize, resolve_devices};
use super::filesystem::StratFilesystem;
use super::metadata::MIN_MDA_SECTORS;

#[derive(Debug)]
pub struct StratPool {
    name: String,
    pool_uuid: Uuid,
    pub cache_devs: BTreeMap<PathBuf, BlockDev>,
    pub block_devs: BTreeMap<PathBuf, BlockDev>,
    pub filesystems: Table<StratFilesystem>,
    redundancy: Redundancy,
}

impl StratPool {
    pub fn new(name: &str,
               paths: &[&Path],
               redundancy: Redundancy,
               force: bool)
               -> EngineResult<StratPool> {
        let pool_uuid = Uuid::new_v4();

        let devices = try!(resolve_devices(paths));
        let bds = try!(initialize(&pool_uuid, devices, MIN_MDA_SECTORS, force));

        let mut pool = StratPool {
            name: name.to_owned(),
            pool_uuid: pool_uuid,
            cache_devs: BTreeMap::new(),
            block_devs: bds,
            filesystems: Table::new(),
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
    fn create_filesystems<'a, 'b>(&'a mut self,
                                  _specs: &[&'b str])
                                  -> EngineResult<Vec<(&'b str, Uuid)>> {
        unimplemented!()
    }

    fn create_snapshot<'a, 'b, 'c>(&'a mut self,
                                   _snapshot_name: &'b str,
                                   _source: &'c Uuid)
                                   -> EngineResult<Uuid> {
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

    fn destroy(mut self) -> EngineResult<()> {

        // TODO: first, tear down DM mappings

        for bd in self.block_devs.values_mut() {
            try!(bd.wipe_metadata());
        }

        for bd in self.cache_devs.values_mut() {
            try!(bd.wipe_metadata());
        }

        Ok(())
    }

    fn destroy_filesystems<'a, 'b>(&'a mut self,
                                   fs_uuids: &[&'b Uuid])
                                   -> EngineResult<Vec<&'b Uuid>> {
        destroy_filesystems!{self; fs_uuids}
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

impl HasUuid for StratPool {
    fn uuid(&self) -> &Uuid {
        &self.pool_uuid
    }
}

impl HasName for StratPool {
    fn name(&self) -> &str {
        &self.name
    }
}

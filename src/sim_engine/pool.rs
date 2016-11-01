// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use bidir_map::BidirMap;

use std::collections::BTreeMap;
use std::iter::FromIterator;
use std::path::Path;
use std::vec::Vec;

use engine::EngineResult;
use engine::Filesystem;
use engine::Pool;

use super::blockdev::SimDev;
use super::filesystem::SimFilesystem;

#[derive(Debug)]
pub struct SimPool {
    pub block_devs: Vec<Box<SimDev>>,
    pub filesystems: BTreeMap<String, Box<SimFilesystem>>,
    pub raid_level: u16,
    pub online: bool,
    pub checking: bool,
}

impl SimPool {
    pub fn new_pool(blockdevs: &[Box<SimDev>], raid_level: u16) -> Box<Pool> {

        let mut vec = Vec::new();
        vec.extend_from_slice(blockdevs);
        let new_pool = SimPool {
            block_devs: vec,
            filesystems: BTreeMap::new(),
            raid_level: raid_level,
            online: true,
            checking: false,
        };

        Box::new(new_pool)
    }
}

impl Pool for SimPool {
    fn add_blockdev(&mut self, _path: &Path) -> EngineResult<()> {
        Ok(())
    }

    fn remove_blockdev(&mut self, _path: &Path) -> EngineResult<()> {
        Ok(())
    }

    fn add_cachedev(&mut self, _path: &Path) -> EngineResult<()> {
        Ok(())
    }

    fn remove_cachedev(&mut self, _path: &Path) -> EngineResult<()> {
        Ok(())
    }

    fn destroy(&mut self) -> EngineResult<()> {
        println!("sim: pool::destroy");
        Ok(())
    }

    fn copy(&self) -> Box<Pool> {
        let pool_copy = SimPool {
            block_devs: self.block_devs.clone(),
            filesystems: self.filesystems.clone(),
            raid_level: self.raid_level.clone(),
            online: true,
            checking: false,
        };
        Box::new(pool_copy)
    }

    fn create_filesystem(&mut self,
                         filesystem_name: &str,
                         mount_point: &str,
                         size: u64)
                         -> EngineResult<()> {
        self.filesystems.insert(filesystem_name.to_owned(),
                                SimFilesystem::new_filesystem(mount_point, size));
        Ok(())
    }
    fn list_filesystems(&self) -> EngineResult<BidirMap<String, Box<Filesystem>>> {
        Ok(BidirMap::from_iter(self.filesystems.iter().map(|x| (x.0.clone(), x.1.copy()))))
    }
}

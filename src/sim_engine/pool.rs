// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;
use std::vec::Vec;

use engine::EngineResult;
use engine::Pool;

use super::blockdev::SimDev;

#[derive(Debug, Clone)]
pub struct SimFilesystem {
    pub name: String,
}

#[derive(Debug)]
pub struct SimPool {
    pub name: String,
    pub block_devs: Vec<Box<SimDev>>,
    pub filesystems: BTreeMap<String, Box<SimFilesystem>>,
    pub raid_level: u16,
    pub online: bool,
    pub checking: bool,
}

impl SimPool {
    pub fn new_pool(name: &str, blockdevs: &[Box<SimDev>], raid_level: u16) -> Box<Pool> {

        let mut vec = Vec::new();
        vec.extend_from_slice(blockdevs);
        let new_pool = SimPool {
            name: name.to_owned(),
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
    fn add_blockdev(&mut self, _path: &str) -> EngineResult<()> {
        println!("sim: pool::add_blockdev");
        Ok(())
    }

    fn add_cachedev(&mut self, _path: &str) -> EngineResult<()> {
        println!("sim: pool::add_cachedev");
        Ok(())
    }

    fn destroy(&mut self) -> EngineResult<()> {
        println!("sim: pool::destroy");
        Ok(())
    }

    fn get_name(&mut self) -> String {
        self.name.clone()
    }
    fn copy(&self) -> Box<Pool> {
        let pool_copy = SimPool {
            name: self.name.clone(),
            block_devs: self.block_devs.clone(),
            filesystems: self.filesystems.clone(),
            raid_level: self.raid_level.clone(),
            online: true,
            checking: false,
        };
        Box::new(pool_copy)
    }
    fn create_filesystem(&mut self,
                         _filesystem_name: &str,
                         _mount_point: &str,
                         _size: u64)
                         -> EngineResult<()> {
        Ok(())
    }
}

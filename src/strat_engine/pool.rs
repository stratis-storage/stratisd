// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;
use std::vec::Vec;
use std::path::Path;

use uuid::Uuid;

use engine::EngineResult;
use engine::Pool;
use engine::Filesystem;
use engine::Dev;
use engine::Cache;

use super::blockdev::BlockDev;
use super::consts::*;

#[derive(Debug, Clone)]
pub struct StratFilesystem {
    pub name: String,
    pub thin_id: u32,
}

#[derive(Debug)]
pub struct StratPool {
    pub name: String,
    pub uuid: String,
    pub cache_devs: Vec<BlockDev>,
    pub block_devs: Vec<BlockDev>,
    pub filesystems: BTreeMap<String, Box<StratFilesystem>>,
    pub raid_level: u16,
    pub online: bool,
    pub checking: bool,
}

impl StratPool {
    pub fn new(name: &str, blockdevs: &[BlockDev], raid_level: u16) -> Box<Pool> {
        Box::new(StratPool {
            name: name.to_owned(),
            uuid: Uuid::new_v4().to_simple_string(),
            cache_devs: Vec::new(),
            block_devs: blockdevs.to_owned(),
            filesystems: BTreeMap::new(),
            raid_level: raid_level,
            online: true,
            checking: false,
        })
    }
}

impl Pool for StratPool {
    fn create_filesystem(&mut self,
                         _filesystem_name: &str,
                         _mount_point: &str,
                         _size: u64)
                         -> EngineResult<()> {
        Ok(())
    }

    fn add_blockdev(&mut self, path: &Path) -> EngineResult<()> {
        let bd = try!(BlockDev::new(&self.uuid, Path::new(path), MIN_MDA_SIZE, true));
        self.block_devs.push(bd);
        Ok(())
    }

    fn add_cachedev(&mut self, _path: &Path) -> EngineResult<()> {
        println!("sim: pool::add_cachedev");
        Ok(())
    }

    fn destroy(&mut self) -> EngineResult<()> {
        println!("sim: pool::destroy");
        Ok(())
    }

    fn copy(&self) -> Box<Pool> {
        let pool_copy = StratPool {
            name: self.name.clone(),
            uuid: self.uuid.clone(),
            cache_devs: self.cache_devs.clone(),
            block_devs: self.block_devs.clone(),
            filesystems: self.filesystems.clone(),
            raid_level: self.raid_level.clone(),
            online: true,
            checking: false,
        };
        Box::new(pool_copy)
    }

    fn list_filesystems(&self) -> EngineResult<BTreeMap<String, Box<Filesystem>>> {
        unimplemented!()
    }

    fn remove_blockdev(&mut self, _path: &Path) -> EngineResult<()> {
        unimplemented!()
    }

    fn remove_cachedev(&mut self, _path: &Path) -> EngineResult<()> {
        unimplemented!()
    }

    fn list_blockdevs(&self) -> EngineResult<Vec<Box<Dev>>> {
        unimplemented!()
    }

    fn list_cachedevs(&self) -> EngineResult<Vec<Box<Cache>>> {
        unimplemented!()
    }
}

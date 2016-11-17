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
    pub pool_uuid: Uuid,
    pub cache_devs: Vec<BlockDev>,
    pub block_devs: Vec<BlockDev>,
    pub filesystems: BTreeMap<String, Box<StratFilesystem>>,
    pub raid_level: u16,
}

impl StratPool {
    pub fn new(name: &str, uuid: Uuid, blockdevs: &[BlockDev], raid_level: u16) -> StratPool {
        StratPool {
            name: name.to_owned(),
            pool_uuid: uuid,
            cache_devs: Vec::new(),
            block_devs: blockdevs.to_owned(),
            filesystems: BTreeMap::new(),
            raid_level: raid_level,
        }
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
        let bd = try!(BlockDev::initialize(&self.pool_uuid, &[path], MIN_MDA_SIZE, true))
            .pop()
            .unwrap();
        self.block_devs.push(bd);
        Ok(())
    }

    fn add_cachedev(&mut self, _path: &Path) -> EngineResult<()> {
        unimplemented!()
    }

    fn destroy_filesystem(&mut self, _filesystem: &str) -> EngineResult<()> {
        unimplemented!()
    }

    fn filesystems(&mut self) -> BTreeMap<&Uuid, &mut Filesystem> {
        unimplemented!()
    }

    fn remove_blockdev(&mut self, _path: &Path) -> EngineResult<()> {
        unimplemented!()
    }

    fn remove_cachedev(&mut self, _path: &Path) -> EngineResult<()> {
        unimplemented!()
    }

    fn blockdevs(&mut self) -> Vec<&mut Dev> {
        unimplemented!()
    }

    fn cachedevs(&mut self) -> Vec<&mut Cache> {
        unimplemented!()
    }

    fn get_filesystem(&mut self, _id: &Uuid) -> EngineResult<&mut Filesystem> {
        unimplemented!()
    }

    fn get_filesystem_id(&mut self, _name: &str) -> EngineResult<Uuid> {
        unimplemented!()
    }
}

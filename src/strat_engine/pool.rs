// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::vec::Vec;
use std::path::Path;
use std::str::FromStr;
use std::iter::FromIterator;
use std::io;
use std::io::ErrorKind;

use uuid::Uuid;
use devicemapper::Device;

use engine::{EngineResult, EngineError};
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
    pub block_devs: BTreeMap<Uuid, BlockDev>,
    pub filesystems: BTreeMap<String, Box<StratFilesystem>>,
    pub raid_level: u16,
}

impl StratPool {
    pub fn new(name: &str,
               devices: BTreeSet<Device>,
               raid_level: u16,
               force: bool)
               -> EngineResult<StratPool> {
        let pool_uuid = Uuid::new_v4();
        let bds = try!(BlockDev::initialize(&pool_uuid, devices, MIN_MDA_SIZE, force));

        Ok(StratPool {
            name: name.to_owned(),
            pool_uuid: pool_uuid,
            cache_devs: Vec::new(),
            block_devs: bds,
            filesystems: BTreeMap::new(),
            raid_level: raid_level,
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
        let dev = try!(Device::from_str(&path.to_string_lossy()));
        let dev_set = BTreeSet::from_iter([dev].iter().map(|x| *x));

        for (_, bd) in &self.block_devs {
            if dev_set.contains(&bd.dev) {
                return Err(EngineError::Io(io::Error::new(ErrorKind::InvalidInput,
                                                          format!("blockdev {} already used \
                                                                   in pool {}",
                                                                  bd.dstr(),
                                                                  self.name))));
            }
        }

        let (uuid, bd) = try!(BlockDev::initialize(&self.pool_uuid, dev_set, MIN_MDA_SIZE, true))
            .into_iter()
            .next()
            .unwrap();
        self.block_devs.insert(uuid, bd);
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

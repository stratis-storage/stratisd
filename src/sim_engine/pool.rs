// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::iter::FromIterator;
use std::path::Path;
use std::rc::Rc;
use std::vec::Vec;

use engine::Cache;
use engine::Dev;
use engine::EngineError;
use engine::EngineResult;
use engine::ErrorEnum;
use engine::Filesystem;
use engine::Pool;

use super::blockdev::SimDev;
use super::cache::SimCacheDev;
use super::filesystem::SimFilesystem;
use super::randomization::Randomizer;

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
    fn add_blockdev(&mut self, path: &Path) -> EngineResult<()> {
        self.block_devs.push(SimDev::new_dev(self.rdm.clone(), path));
        Ok(())
    }

    fn add_cachedev(&mut self, path: &Path) -> EngineResult<()> {
        self.cache_devs.push(SimCacheDev::new_cache(self.rdm.clone(), path));
        Ok(())
    }

    fn destroy_filesystem(&mut self, filesystem: &str) -> EngineResult<()> {
        match self.filesystems.remove(filesystem) {
            Some(_) => {
                return Ok(());
            }
            None => return Err(EngineError::Stratis(ErrorEnum::NotFound(filesystem.into()))),
        }
    }

    fn create_filesystem(&mut self,
                         filesystem_name: &str,
                         mount_point: &str,
                         size: u64)
                         -> EngineResult<()> {

        if self.filesystems.contains_key(filesystem_name) {
            return Err(EngineError::Stratis(ErrorEnum::AlreadyExists(filesystem_name.into())));
        }

        self.filesystems.insert(filesystem_name.to_owned(),
                                SimFilesystem::new_filesystem(mount_point, size));
        Ok(())
    }

    fn filesystems(&self) -> EngineResult<BTreeMap<String, &Filesystem>> {
        Ok(BTreeMap::from_iter(self.filesystems.iter().map(|x| (x.0.clone(), x.1 as &Filesystem))))
    }

    fn blockdevs(&self) -> EngineResult<Vec<&Dev>> {
        Ok(Vec::from_iter(self.block_devs.iter().map(|x| x as &Dev)))
    }

    fn cachedevs(&self) -> EngineResult<Vec<&Cache>> {
        Ok(Vec::from_iter(self.cache_devs.iter().map(|x| x as &Cache)))
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
}

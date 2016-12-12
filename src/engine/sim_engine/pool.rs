// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::iter::FromIterator;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::vec::Vec;

use bidir_map::BidirMap;

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

use uuid::Uuid;

#[derive(Debug)]
pub struct SimPool {
    pub block_devs: Vec<SimDev>,
    pub cache_devs: Vec<SimCacheDev>,
    pub filesystems: BidirMap<Uuid, SimFilesystem>,
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
            filesystems: BidirMap::new(),
            cache_devs: Vec::new(),
            raid_level: raid_level,
            rdm: rdm,
        };

        new_pool
    }

}

impl Pool for SimPool {
    fn add_blockdevs(&mut self, paths: &[&Path], _force: bool) -> EngineResult<Vec<PathBuf>> {
        let rdm = self.rdm.clone();
        let devices = BTreeSet::from_iter(paths);
        self.block_devs.extend(devices.iter().map(|p| SimDev::new_dev(rdm.clone(), p)));
        Ok(devices.iter().map(|d| d.to_path_buf()).collect())
    }

    fn add_cachedevs(&mut self, paths: &[&Path], _force: bool) -> EngineResult<Vec<PathBuf>> {
        let rdm = self.rdm.clone();
        let devices = BTreeSet::from_iter(paths);
        self.cache_devs.extend(devices.iter().map(|p| SimCacheDev::new_cache(rdm.clone(), p)));
        Ok(devices.iter().map(|d| d.to_path_buf()).collect())
    }

    fn destroy_filesystem(&mut self, name: &str) -> EngineResult<()> {

        match self.get_filesystem_id(name) {
            Ok(filesystem_id) => {
                match self.filesystems.remove_by_first(&filesystem_id) {
                    Some(_) => {
                        return Ok(());
                    }
                    None => {
                        return Err(EngineError::Stratis(ErrorEnum::NotFound(filesystem_id.simple()
                            .to_string())))
                    }
                }
            }
            Err(err) => {
                return Err(err);
            }
        }
        Ok(())
    }

    fn create_filesystem(&mut self,
                         name: &str,
                         mount_point: &str,
                         quota_size: Option<u64>)
                         -> EngineResult<Uuid> {

        match self.get_filesystem_id(name) {
            Ok(_) => {
                return Err(EngineError::Stratis(ErrorEnum::AlreadyExists(String::from(name))));
            }
            Err(_) => {}
        }

        let new_filesystem = SimFilesystem::new_filesystem(name, mount_point, quota_size);
        let fs_uuid = new_filesystem.get_id();
        self.filesystems.insert(fs_uuid, new_filesystem);
        Ok(fs_uuid)
    }

    fn create_snapshot(&mut self, snapshot_name: &str, source: &str) -> EngineResult<()> {

        let parent_id = try!(self.get_filesystem_id(source));

        let uuid = try!(self.create_filesystem(&snapshot_name, &String::from(""), None));

        let new_snapshot = try!(self.get_filesystem_by_id(&uuid));

        new_snapshot.add_ancestor(parent_id);

        Ok(())
    }

    fn filesystems(&mut self) -> BTreeMap<&Uuid, &mut Filesystem> {
        BTreeMap::from_iter(self.filesystems
            .iter_mut()
            .map(|x| (&x.0 as &Uuid, &mut x.1 as &mut Filesystem)))
    }

    fn blockdevs(&mut self) -> Vec<&mut Dev> {
        Vec::from_iter(self.block_devs.iter_mut().map(|x| x as &mut Dev))
    }

    fn cachedevs(&mut self) -> Vec<&mut Cache> {
        Vec::from_iter(self.cache_devs.iter_mut().map(|x| x as &mut Cache))
    }

    fn get_filesystem_by_id(&mut self, id: &Uuid) -> EngineResult<&mut Filesystem> {
        get_filesystem_by_id!(self; id)
    }

    fn get_filesystem_id(&self, name: &str) -> EngineResult<Uuid> {
        get_filesystem_id!(self; name)
    }

    fn get_filesystem_by_name(&mut self, name: &str) -> EngineResult<&mut Filesystem> {
        get_filesystem_by_name!(self; name)
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

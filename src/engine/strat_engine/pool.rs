// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use devicemapper::DM;

use std::collections::{HashMap, HashSet};
use std::collections::hash_map::RandomState;
use std::iter::FromIterator;
use std::path::Path;
use std::path::PathBuf;
use std::vec::Vec;

use time::{now, Timespec};
use uuid::Uuid;
use serde_json;

use engine::EngineError;
use engine::EngineResult;
use engine::ErrorEnum;
use engine::Filesystem;
use engine::Pool;
use engine::RenameAction;
use engine::engine::Redundancy;
use engine::strat_engine::device::wipe_sectors;
use engine::strat_engine::lineardev::LinearDev;
use engine::strat_engine::thinpooldev::ThinPoolDev;

use super::super::engine::{FilesystemUuid, HasName, HasUuid};
use super::super::structures::Table;

use super::serde_structs::StratSave;
use super::blockdev::{BlockDev, initialize};
use super::device::resolve_devices;
use super::filesystem::StratFilesystem;
use super::metadata::MIN_MDA_SECTORS;

use types::{DataBlocks, Sectors};

pub const DATA_BLOCK_SIZE: Sectors = Sectors(2048);

#[derive(Debug)]
pub struct StratPool {
    name: String,
    pool_uuid: Uuid,
    pub block_devs: HashMap<PathBuf, BlockDev>,
    pub filesystems: Table<StratFilesystem>,
    redundancy: Redundancy,
    thin_pool: ThinPoolDev,
}

impl StratPool {
    pub fn new(name: &str,
               dm: &DM,
               paths: &[&Path],
               redundancy: Redundancy,
               force: bool)
               -> EngineResult<StratPool> {
        let pool_uuid = Uuid::new_v4();

        let devices = try!(resolve_devices(paths));
        let bds = try!(initialize(&pool_uuid, devices, MIN_MDA_SECTORS, force));

        // TODO: We've got some temporary code in BlockDev::initialize that
        // makes sure we've got at least 2 blockdevs supplied - one for a meta
        // and one for data.  In the future, we will be able to deal with a
        // single blockdev.  When that code is added to use a single blockdev,
        // the check for 2 devs in BlockDev::initialize should be removed.
        assert!(bds.len() >= 2);

        let meta_dev = try!(LinearDev::new(&format!("stratis_{}_meta", name), dm, &vec![&bds[0]]));
        // When constructing a thin-pool, Stratis reserves the first N
        // sectors on a block device by creating a linear device with a
        // starting offset. DM writes the super block in the first block.
        // DM requires this first block to be zeros when the meta data for
        // the thin-pool is initially created. If we don't zero the
        // superblock DM issue error messages because it triggers code paths
        // that are trying to re-adopt the device with the attributes that
        // have been passed.
        try!(wipe_sectors(&try!(meta_dev.path()), Sectors(0), DATA_BLOCK_SIZE));
        let data_dev = try!(LinearDev::new(&format!("stratis_{}_data", name),
                                           dm,
                                           &Vec::from_iter(bds[1..].iter())));
        try!(wipe_sectors(&try!(data_dev.path()), Sectors(0), DATA_BLOCK_SIZE));
        let length = try!(data_dev.size()).sectors();

        // TODO Fix hard coded data blocksize and low water mark.
        let thinpool_dev = try!(ThinPoolDev::new(&format!("stratis_{}_thinpool", name),
                                                 dm,
                                                 length,
                                                 DATA_BLOCK_SIZE,
                                                 DataBlocks(256000),
                                                 meta_dev,
                                                 data_dev));

        let mut blockdevs = HashMap::new();
        for bd in bds {
            blockdevs.insert(bd.devnode.clone(), bd);
        }
        let mut pool = StratPool {
            name: name.to_owned(),
            pool_uuid: pool_uuid,
            block_devs: blockdevs,
            filesystems: Table::new(),
            redundancy: redundancy,
            thin_pool: thinpool_dev,
        };

        try!(pool.write_metadata());

        Ok(pool)
    }

    /// Return the metadata from the first blockdev with up-to-date, readable
    /// metadata.
    /// Precondition: All Blockdevs in blockdevs must belong to the same pool.
    pub fn load_state(blockdevs: &[&BlockDev]) -> Option<Vec<u8>> {
        if blockdevs.is_empty() {
            return None;
        }

        let most_recent_blockdev = blockdevs.iter()
            .max_by_key(|bd| bd.last_update_time())
            .expect("must be a maximum since bds is non-empty");

        let most_recent_time = most_recent_blockdev.last_update_time();

        if most_recent_time.is_none() {
            return None;
        }

        for bd in blockdevs.iter().filter(|b| b.last_update_time() == most_recent_time) {
            match bd.load_state() {
                Ok(Some(data)) => return Some(data),
                _ => continue,
            }
        }

        None
    }

    /// Write the given data to all blockdevs marking with specified time.
    pub fn save_state(devs: &mut [&mut BlockDev],
                      time: &Timespec,
                      metadata: &[u8])
                      -> EngineResult<()> {
        // TODO: Do something better than panic when saving to blockdev fails.
        for bd in devs {
            bd.save_state(time, metadata).unwrap();
        }
        Ok(())
    }

    /// Write pool metadata to all its blockdevs marking with current time.

    // TODO: Cap # of blockdevs written to, as described in SWDD

    // TODO: Check current time against global last updated, and use
    // alternate time value if earlier, as described in SWDD
    pub fn write_metadata(&mut self) -> EngineResult<()> {
        let data = try!(serde_json::to_string(&self.to_save()));
        let mut blockdevs: Vec<&mut BlockDev> = self.block_devs.values_mut().collect();
        StratPool::save_state(&mut blockdevs, &now().to_timespec(), data.as_bytes())
    }

    pub fn to_save(&self) -> StratSave {
        StratSave {
            name: self.name.clone(),
            id: self.pool_uuid.simple().to_string(),
            block_devs: self.block_devs
                .iter()
                .map(|(_, bd)| (bd.uuid().simple().to_string(), bd.to_save()))
                .collect(),
        }
    }
}

impl Pool for StratPool {
    fn create_filesystems<'a, 'b>(&'a mut self,
                                  specs: &[&'b str])
                                  -> EngineResult<Vec<(&'b str, FilesystemUuid)>> {
        let names: HashSet<_, RandomState> = HashSet::from_iter(specs);
        for name in names.iter() {
            if self.filesystems.contains_name(name) {
                return Err(EngineError::Engine(ErrorEnum::AlreadyExists, name.to_string()));
            }
        }

        let mut result = Vec::new();
        for name in names.iter() {
            let uuid = Uuid::new_v4();
            let dm = try!(DM::new());
            let new_filesystem = try!(StratFilesystem::new(uuid, name, &dm, &mut self.thin_pool));
            self.filesystems.insert(new_filesystem);
            result.push((**name, uuid));
        }

        Ok(result)
    }

    fn add_blockdevs(&mut self, paths: &[&Path], force: bool) -> EngineResult<Vec<PathBuf>> {
        let devices = try!(resolve_devices(paths));
        let bds = try!(initialize(&self.pool_uuid, devices, MIN_MDA_SECTORS, force));
        let bdev_paths = bds.iter().map(|p| p.devnode.clone()).collect();
        for bd in bds {
            self.block_devs.insert(bd.devnode.clone(), bd);
        }
        try!(self.write_metadata());
        Ok(bdev_paths)
    }

    fn destroy(self) -> EngineResult<()> {

        // TODO Do we want to create a new File each time we interact with DM?
        let dm = try!(DM::new());
        try!(self.thin_pool.teardown(&dm));

        for bd in self.block_devs.values() {
            try!(bd.wipe_metadata());
        }

        Ok(())
    }

    fn destroy_filesystems<'a, 'b>(&'a mut self,
                                   fs_uuids: &[&'b FilesystemUuid])
                                   -> EngineResult<Vec<&'b FilesystemUuid>> {
        destroy_filesystems!{self; fs_uuids}
    }

    fn rename_filesystem(&mut self,
                         uuid: &FilesystemUuid,
                         new_name: &str)
                         -> EngineResult<RenameAction> {
        rename_filesystem!{self; uuid; new_name}
    }

    fn rename(&mut self, name: &str) {
        self.name = name.to_owned();
    }

    fn get_filesystem(&mut self, uuid: &FilesystemUuid) -> Option<&mut Filesystem> {
        get_filesystem!(self; uuid)
    }
}

impl HasUuid for StratPool {
    fn uuid(&self) -> &FilesystemUuid {
        &self.pool_uuid
    }
}

impl HasName for StratPool {
    fn name(&self) -> &str {
        &self.name
    }
}

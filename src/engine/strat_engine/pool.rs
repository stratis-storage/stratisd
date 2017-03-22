// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use devicemapper::DM;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::iter::FromIterator;
use std::path::Path;
use std::path::PathBuf;
use std::vec::Vec;

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
use engine::strat_engine::lineardev::LinearDev;
use engine::strat_engine::thinpooldev::ThinPoolDev;

use super::super::engine::{DevUuid, FilesystemUuid, HasName, HasUuid};
use super::super::structures::Table;

use super::serde_structs::StratSave;
use super::blockdev::{BlockDev, initialize, resolve_devices};
use super::filesystem::StratFilesystem;
use super::metadata::MIN_MDA_SECTORS;

use types::DataBlocks;
use types::Sectors;

#[derive(Debug)]
pub struct StratPool {
    name: String,
    pool_uuid: Uuid,
    pub block_devs: BTreeMap<PathBuf, BlockDev>,
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
        let meta_dev;
        let data_dev;
        let length;
        {
            let mut data_devs = Vec::from_iter(bds.values());

            meta_dev = try!(LinearDev::new(&format!("stratis_{}_meta", name),
                                           dm,
                                           &vec![data_devs.remove(0)]));

            data_dev = try!(LinearDev::new(&format!("stratis_{}_data", name), dm, &data_devs));
            length = try!(data_dev.size()).sectors();

        }
        // TODO Fix hard coded data blocksize and low water mark.
        let thinpool_dev = try!(ThinPoolDev::new(&format!("stratis_{}_thinpool", name),
                                                 dm,
                                                 length,
                                                 Sectors(1024),
                                                 DataBlocks(256000),
                                                 meta_dev,
                                                 data_dev));

        let mut pool = StratPool {
            name: name.to_owned(),
            pool_uuid: pool_uuid,
            block_devs: bds,
            filesystems: Table::new(),
            redundancy: redundancy,
            thin_pool: thinpool_dev,
        };

        try!(pool.write_metadata());

        Ok(pool)
    }

    /// Return the metadata from the first blockdev with up-to-date, readable
    /// metadata.
    pub fn read_metadata(&self) -> Option<Vec<u8>> {

        let mut bds: Vec<&BlockDev> = self.block_devs
            .iter()
            .map(|(_, bd)| bd)
            .filter(|bd| bd.last_update_time().is_some())
            .collect();

        if bds.is_empty() {
            return None;
        }

        bds.sort_by_key(|k| {
            k.last_update_time().expect("devs without some last update time filtered above.")
        });

        let last_update_time = bds.last()
            .expect("There is a last bd, since bds.is_empty() was false.")
            .last_update_time()
            .expect("devs without some last update time filtered above.");

        for bd in bds.iter()
            .rev()
            .take_while(|bd| {
                bd.last_update_time()
                    .expect("devs without some last update time filtered above.") ==
                last_update_time
            }) {
            match bd.load_state() {
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

        // TODO: Do something better than panic when saving to blockdev fails.
        for (_, bd) in &mut self.block_devs {
            bd.save_state(&time, data.as_bytes()).unwrap();
        }

        Ok(())
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
        let names = BTreeSet::from_iter(specs);
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

    fn add_blockdevs(&mut self, paths: &[&Path], force: bool) -> EngineResult<Vec<DevUuid>> {
        let devices = try!(resolve_devices(paths));
        let mut bds = try!(initialize(&self.pool_uuid, devices, MIN_MDA_SECTORS, force));
        let bdev_uuids = bds.iter().map(|p| p.1.uuid().clone()).collect();
        self.block_devs.append(&mut bds);
        try!(self.write_metadata());
        Ok(bdev_uuids)
    }

    fn destroy(mut self) -> EngineResult<()> {

        // TODO Do we want to create a new File each time we interact with DM?
        let dm = try!(DM::new());
        try!(self.thin_pool.teardown(&dm));

        for bd in self.block_devs.values_mut() {
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

    fn set_name(&mut self, name: &str) {
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

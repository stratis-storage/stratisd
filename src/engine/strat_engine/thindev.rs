// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use devicemapper::{DM, DevId, DeviceInfo, DmFlags};
use engine::{EngineError, EngineResult, ErrorEnum};
use engine::strat_engine::blockdev::BlockDev;
use engine::strat_engine::thinpooldev::ThinPoolDev;

use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use types::Sectors;

#[derive(Clone)]
pub struct ThinDev {
    name: String,
    pub dev_info: DeviceInfo,
    pub thin_id: u32,
    pub size: Sectors,
}

impl fmt::Debug for ThinDev {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.name())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ThinStatus {
    Good(Sectors),
    Fail,
}

/// support use of DM for thin provisioned devices over pools
impl ThinDev {
    /// Use the given ThinPoolDev as backing space for a newly constructed
    /// thin provisioned ThinDev returned by new().
    pub fn new(name: &str,
               dm: &DM,
               thin_pool: &mut ThinPoolDev,
               thin_id: u32,
               length: Sectors)
               -> EngineResult<ThinDev> {

        let thin_dev_path = format!("{}", try!(thin_pool.path()).to_string_lossy());
        try!(thin_pool.message(dm, &format!("create_thin {}", thin_id)));
        try!(dm.device_create(name, None, DmFlags::empty()));
        let table = ThinDev::dm_table(&thin_dev_path, thin_id, &length);
        let id = &DevId::Name(name);
        let di = try!(dm.table_load(id, &table));
        try!(dm.device_suspend(id, DmFlags::empty()));
        BlockDev::wait_for_dm();
        Ok(ThinDev {
            name: name.to_owned(),
            dev_info: di,
            thin_id: thin_id,
            size: length,
        })
    }

    /// Generate a Vec<> to be passed to DM.  The format of the Vec entries are:
    /// "<start> <length> thin </dev/mapper/poolname> <thin_id>"
    fn dm_table(pool_dev_str: &String,
                thin_id: u32,
                length: &Sectors)
                -> Vec<(u64, u64, String, String)> {
        let mut table = Vec::new();

        let params = format!("{} {}", pool_dev_str, thin_id);

        table.push((0u64, length.0, "thin".to_owned(), params));
        debug!("dmtable line : {:?}", table);
        table

    }

    pub fn teardown(&mut self, dm: &DM) -> EngineResult<()> {
        try!(dm.device_remove(&DevId::Name(&self.name), DmFlags::empty()));
        Ok(())
    }

    pub fn name(&self) -> &str {
        self.dev_info.name()
    }

    pub fn path(&self) -> EngineResult<PathBuf> {
        match self.dev_info.device().path() {
            Some(path) => return Ok(path),
            None => {
                return Err(EngineError::Engine(ErrorEnum::Invalid,
                                               "No path associated with dev_info".into()))
            }
        }
    }

    pub fn create_fs(&mut self) -> EngineResult<()> {
        let path_result = try!(self.path());
        let dev_path = Path::new(&path_result);

        debug!("Create filesystem for : {:?}", dev_path);
        let output = try!(Command::new("mkfs.xfs")
            .arg("-f")
            .arg(dev_path)
            .output());

        if output.status.success() {
            debug!("Created xfs filesystem on {:?}", dev_path)
        } else {
            let message = String::from_utf8_lossy(&output.stderr);
            debug!("stderr: {}", message);
            return Err(EngineError::Engine(ErrorEnum::Error, message.into()));
        }
        Ok(())
    }

    pub fn mount_fs(&mut self, mount_point: &Path) -> EngineResult<()> {
        let path_result = try!(self.path());
        let dev_path = Path::new(&path_result);

        debug!("Mount filesystem {:?} on : {:?}", dev_path, mount_point);
        let output = try!(Command::new("mount")
            .arg(dev_path)
            .arg(mount_point)
            .output());

        if output.status.success() {
            debug!("Mounted xfs filesystem on {:?}", mount_point)
        } else {
            let message = String::from_utf8_lossy(&output.stderr);
            debug!("stderr: {}", message);
            return Err(EngineError::Engine(ErrorEnum::Error, message.into()));
        }
        Ok(())
    }

    pub fn unmount_fs(&mut self, mount_point: &Path) -> EngineResult<()> {
        debug!("Unount filesystem {:?}", mount_point);

        let output = try!(Command::new("umount")
            .arg(mount_point)
            .output());

        if output.status.success() {
            debug!("Unmounted filesystem {:?}", mount_point)
        } else {
            let message = String::from_utf8_lossy(&output.stderr);
            debug!("stderr: {}", message);
            return Err(EngineError::Engine(ErrorEnum::Error, message.into()));
        }
        Ok(())
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
extern crate rand;

use std::error::Error;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use devicemapper::DM;
use devicemapper::Bytes;
use devicemapper::{ThinDev, ThinStatus};
use devicemapper::ThinPoolDev;

use super::super::consts::IEC;
use super::super::engine::{Filesystem, HasName, HasUuid};
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::types::{FilesystemUuid, PoolUuid};

use super::dmdevice::{ThinRole, format_thin_name};
use super::serde_structs::{FilesystemSave, Recordable};

#[derive(Debug)]
pub struct StratFilesystem {
    fs_id: FilesystemUuid,
    name: String,
    thin_dev: ThinDev,
}

pub enum FilesystemStatus {
    Good,
    Failed,
}

impl StratFilesystem {
    pub fn initialize(pool_id: &PoolUuid,
                      fs_id: FilesystemUuid,
                      name: &str,
                      dm: &DM,
                      thin_pool: &ThinPoolDev)
                      -> EngineResult<StratFilesystem> {
        // TODO should replace with proper id generation. DM takes a 24 bit
        // number for the thin_id.  Generate a u16 to avoid the possibility of
        // "too big". Should this be moved into the DM binding (or lower)?
        // How can a client be expected to avoid collisions?
        let thin_id = rand::random::<u16>();
        let device_name = format_thin_name(pool_id, ThinRole::Filesystem(fs_id));
        // TODO We don't require a size to be provided for create_filesystems -
        // but devicemapper requires an initial size for a thin provisioned
        // device - currently hard coded to 1GB.
        let new_thin_dev = try!(ThinDev::new(&device_name,
                                             dm,
                                             thin_pool,
                                             thin_id as u32,
                                             Bytes(IEC::Ti).sectors()));
        try!(create_fs(try!(new_thin_dev.devnode()).as_path()));
        Ok(StratFilesystem {
               fs_id: fs_id,
               name: name.to_owned(),
               thin_dev: new_thin_dev,
           })
    }

    pub fn check(&self, dm: &DM) -> EngineResult<FilesystemStatus> {
        match try!(self.thin_dev.status(dm)) {
            ThinStatus::Good((_mapped, _highest)) => {
                // TODO: check if filesystem is getting full and might need to
                // be extended (hint: use statfs(2))
                // TODO: periodically kick off fstrim?
            }
            ThinStatus::Fail => return Ok(FilesystemStatus::Failed),
        }
        Ok(FilesystemStatus::Good)
    }
}

impl HasName for StratFilesystem {
    fn name(&self) -> &str {
        &self.name
    }
}

impl HasUuid for StratFilesystem {
    fn uuid(&self) -> &FilesystemUuid {
        &self.fs_id
    }
}

impl Filesystem for StratFilesystem {
    fn rename(&mut self, name: &str) {
        self.name = name.to_owned();
    }

    fn destroy(self) -> EngineResult<()> {
        let dm = try!(DM::new());
        match self.thin_dev.teardown(&dm) {
            Ok(_) => Ok(()),
            Err(e) => Err(EngineError::Engine(ErrorEnum::Error, e.description().into())),
        }
    }

    fn devnode(&self) -> EngineResult<PathBuf> {
        Ok(try!(self.thin_dev.devnode()))
    }
}

impl Recordable<FilesystemSave> for StratFilesystem {
    fn record(&self) -> EngineResult<FilesystemSave> {
        Ok(FilesystemSave {
               name: self.name.clone(),
               uuid: self.fs_id,
               thin_id: self.thin_dev.id(),
               size: self.thin_dev.size(),
           })
    }
}

pub fn create_fs(dev_path: &Path) -> EngineResult<()> {

    debug!("Create filesystem for : {:?}", dev_path);
    let output = try!(Command::new("mkfs.xfs")
                          .arg("-f")
                          .arg(&dev_path)
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

pub fn mount_fs(dev_path: &Path, mount_point: &Path) -> EngineResult<()> {

    debug!("Mount filesystem {:?} on : {:?}", dev_path, mount_point);
    let output = try!(Command::new("mount")
                          .arg(&dev_path)
                          .arg(mount_point)
                          .output());

    if output.status.success() {
        debug!("Mounted filesystem on {:?}", mount_point)
    } else {
        let message = String::from_utf8_lossy(&output.stderr);
        debug!("stderr: {}", message);
        return Err(EngineError::Engine(ErrorEnum::Error, message.into()));
    }
    Ok(())
}

pub fn unmount_fs<I, S>(mount_point: &Path, flags: I) -> EngineResult<()>
    where I: IntoIterator<Item = S>,
          S: AsRef<OsStr>
{
    debug!("Unmount filesystem {:?}", mount_point);

    let mut command = Command::new("umount");
    let output = try!(command.arg(mount_point).args(flags).output());

    if output.status.success() {
        debug!("Unmounted filesystem {:?}", mount_point)
    } else {
        let message = String::from_utf8_lossy(&output.stderr);
        debug!("stderr: {}", message);
        return Err(EngineError::Engine(ErrorEnum::Error, message.into()));
    }
    Ok(())
}

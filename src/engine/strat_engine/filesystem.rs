// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

use devicemapper::DM;
use devicemapper::Sectors;
use devicemapper::{ThinDev, ThinDevId, ThinStatus};

use super::super::engine::{Filesystem, HasName, HasUuid};
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::types::{FilesystemUuid, PoolUuid};

use super::dmdevice::{ThinRole, format_thin_name};
use super::serde_structs::{FilesystemSave, Recordable};
use super::thinpool::ThinPool;

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
    pub fn initialize(pool_uuid: &PoolUuid,
                      fs_id: FilesystemUuid,
                      name: &str,
                      dm: &DM,
                      thin_pool: &mut ThinPool)
                      -> EngineResult<StratFilesystem> {
        let device_name = format_thin_name(pool_uuid, ThinRole::Filesystem(fs_id));
        let thin_dev = try!(thin_pool.make_thin_device(dm, &device_name));
        let fs = StratFilesystem {
            fs_id: fs_id,
            name: name.to_owned(),
            thin_dev: thin_dev,
        };

        try!(create_fs(try!(fs.devnode()).as_path()));
        Ok(fs)
    }

    /// Setup a filesystem, setting up the thin device as necessary.
    // FIXME: Check for still existing device mapper devices.
    pub fn setup(pool_uuid: PoolUuid,
                 fs_id: FilesystemUuid,
                 thindev_id: ThinDevId,
                 name: &str,
                 size: Sectors,
                 dm: &DM,
                 thin_pool: &ThinPool)
                 -> EngineResult<StratFilesystem> {
        let device_name = format_thin_name(&pool_uuid, ThinRole::Filesystem(fs_id));
        let thin_dev = try!(thin_pool.setup_thin_device(dm, &device_name, thindev_id, size));

        Ok(StratFilesystem {
               fs_id: fs_id,
               name: name.to_owned(),
               thin_dev: thin_dev,
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

    /// The thin id for the thin device that backs this filesystem.
    pub fn thin_id(&self) -> ThinDevId {
        self.thin_dev.id()
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

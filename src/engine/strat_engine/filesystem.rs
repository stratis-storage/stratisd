// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

use devicemapper::DM;
use devicemapper::{ThinDev, ThinDevId, ThinStatus};

use super::super::engine::{Filesystem, HasName, HasUuid};
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::types::FilesystemUuid;

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
    pub fn initialize(fs_id: FilesystemUuid,
                      name: &str,
                      thin_dev: ThinDev)
                      -> EngineResult<StratFilesystem> {

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
    pub fn setup(fs_id: FilesystemUuid,
                 name: &str,
                 thin_dev: ThinDev)
                 -> EngineResult<StratFilesystem> {

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

    /// Tear down the filesystem.
    pub fn teardown(self, dm: &DM) -> EngineResult<()> {
        Ok(try!(self.thin_dev.teardown(dm)))
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

/// Create a filesystem on devnode.
pub fn create_fs(devnode: &Path) -> EngineResult<()> {
    if try!(Command::new("mkfs.xfs")
                .arg("-f")
                .arg(&devnode)
                .status())
               .success() {
        Ok(())
    } else {
        let err_msg = format!("Failed to create new filesystem at {:?}", devnode);
        Err(EngineError::Engine(ErrorEnum::Error, err_msg))
    }
}

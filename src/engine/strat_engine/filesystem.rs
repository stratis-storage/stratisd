// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::{Path, PathBuf};
use std::process::Command;

use mnt::get_mount;

use devicemapper::DM;
use devicemapper::{ThinDev, ThinDevId, ThinStatus, ThinPoolDev};

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
    /// Create a StratFilesystem on top of the given ThinDev.
    pub fn initialize(fs_id: FilesystemUuid,
                      name: &str,
                      thin_dev: ThinDev)
                      -> EngineResult<StratFilesystem> {
        let fs = StratFilesystem::setup(fs_id, name, thin_dev);

        try!(create_fs(try!(fs.devnode()).as_path()));
        Ok(fs)
    }

    /// Build a StratFilesystem that includes the ThinDev and related info.
    pub fn setup(fs_id: FilesystemUuid, name: &str, thin_dev: ThinDev) -> StratFilesystem {
        StratFilesystem {
            fs_id: fs_id,
            name: name.to_owned(),
            thin_dev: thin_dev,
        }
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

    /// Get the mount_point for this filesystem
    pub fn get_mount_point(&self) -> EngineResult<PathBuf> {
        match get_mount(&try!(self.devnode())) {
            Ok(list) => {
                match list {
                    Some(mount) => Ok(mount.file),
                    None => {
                        Err(EngineError::Engine(ErrorEnum::Error,
                                                "No mount point for filesystem".into()))
                    }
                }
            }
            Err(e) => Err(EngineError::Engine(ErrorEnum::Error, e.description().into())),
        }
    }

    /// Tear down the filesystem.
    pub fn teardown(self, dm: &DM) -> EngineResult<()> {
        Ok(try!(self.thin_dev.teardown(dm)))
    }

    /// Set the name of this filesystem to name.
    pub fn rename(&mut self, name: &str) {
        self.name = name.to_owned();
    }

    /// Destroy the filesystem.
    pub fn destroy(self, dm: &DM, thin_pool: &ThinPoolDev) -> EngineResult<()> {
        Ok(try!(self.thin_dev.destroy(dm, thin_pool)))
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

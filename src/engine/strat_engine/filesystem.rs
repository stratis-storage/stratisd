// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::{Path, PathBuf};
use std::process::Command;

use devicemapper::{Bytes, DM, Sectors, ThinDev, ThinDevId, ThinStatus, ThinPoolDev};
use devicemapper::consts::{IEC, SECTOR_SIZE};

use nix::sys::statvfs::statvfs;
use nix::sys::statvfs::vfs::Statvfs;

use super::super::engine::{Filesystem, HasName, HasUuid};
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::types::FilesystemUuid;

use super::serde_structs::{FilesystemSave, Recordable};

/// TODO: confirm that 256 MiB leaves enough time for stratisd to respond and extend before
/// the filesystem is out of space.
pub const FILESYSTEM_LOWATER: Sectors = Sectors(256 * IEC::Mi / (SECTOR_SIZE as u64)); // = 256 MiB

#[derive(Debug)]
pub struct StratFilesystem {
    fs_id: FilesystemUuid,
    name: String,
    thin_dev: ThinDev,
}

pub enum FilesystemStatus {
    Good,
    XfsGrowFailed,
    ThinDevExtendFailed,
    Failed,
}

impl StratFilesystem {
    /// Create a StratFilesystem on top of the given ThinDev.
    pub fn initialize(fs_id: FilesystemUuid,
                      name: &str,
                      thin_dev: ThinDev)
                      -> EngineResult<StratFilesystem> {
        let fs = StratFilesystem::setup(fs_id, name, thin_dev);

        create_fs(fs.devnode()?.as_path())?;
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

    /// check if filesystem is getting full and needs to be extended
    /// TODO: deal with the thindev in a Fail state.
    pub fn check(&mut self, dm: &DM) -> EngineResult<FilesystemStatus> {
        match self.thin_dev.status(dm)? {
            ThinStatus::Good(_) => {
                let mount_point = self.get_mount_point()?;
                let (fs_total_bytes, fs_total_used_bytes) = fs_usage(&mount_point)?;
                let free_bytes = fs_total_bytes - fs_total_used_bytes;
                if free_bytes.sectors() < FILESYSTEM_LOWATER {
                    let extend_size = self.extend_size(self.thin_dev.size());
                    if self.thin_dev.extend(dm, extend_size).is_err() {
                        return Ok(FilesystemStatus::ThinDevExtendFailed);
                    }
                    if xfs_growfs(&mount_point).is_err() {
                        return Ok(FilesystemStatus::XfsGrowFailed);
                    }
                }
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

    /// Return an extend size for the thindev under the filesystem
    /// TODO: returning the current size will double the space provisoned to
    /// the thin device.  We should determine if this is a reasonable value.
    fn extend_size(&self, current_size: Sectors) -> Sectors {
        current_size
    }

    /// Get the mount_point for this filesystem
    /// TODO Replace this code with something less brittle
    pub fn get_mount_point(&self) -> EngineResult<PathBuf> {
        let output = Command::new("df")
            .arg("--output=target")
            .arg(&self.devnode()?)
            .output()?;
        if output.status.success() {
            let output_str = String::from_utf8_lossy(&output.stdout);
            if let Some(mount_point) = output_str.lines().last() {
                return Ok(PathBuf::from(mount_point));
            }
        }
        let err_msg = format!("Failed to get filesystem mountpoint at {:?}",
                              self.devnode());
        Err(EngineError::Engine(ErrorEnum::Error, err_msg))
    }

    /// Tear down the filesystem.
    pub fn teardown(self, dm: &DM) -> EngineResult<()> {
        Ok(self.thin_dev.teardown(dm)?)
    }

    /// Set the name of this filesystem to name.
    pub fn rename(&mut self, name: &str) {
        self.name = name.to_owned();
    }

    /// Destroy the filesystem.
    pub fn destroy(self, dm: &DM, thin_pool: &ThinPoolDev) -> EngineResult<()> {
        Ok(self.thin_dev.destroy(dm, thin_pool)?)
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
        Ok(self.thin_dev.devnode()?)
    }
}

impl Recordable<FilesystemSave> for StratFilesystem {
    fn record(&self) -> FilesystemSave {
        FilesystemSave {
            name: self.name.clone(),
            uuid: self.fs_id,
            thin_id: self.thin_dev.id(),
            size: self.thin_dev.size(),
        }
    }
}

/// Return total bytes allocated to the filesystem, total bytes used by data/metadata
pub fn fs_usage(mount_point: &Path) -> EngineResult<(Bytes, Bytes)> {
    let mut stat = Statvfs::default();
    statvfs(mount_point, &mut stat)?;
    Ok((Bytes(stat.f_bsize * stat.f_blocks), Bytes(stat.f_bsize * (stat.f_blocks - stat.f_bfree))))
}

/// Use the xfs_growfs command to expand a filesystem mounted at the given
/// mount point.
pub fn xfs_growfs(mount_point: &Path) -> EngineResult<()> {
    if Command::new("xfs_growfs")
           .arg(mount_point)
           .arg("-d")
           .status()?
           .success() {
        Ok(())
    } else {
        let err_msg = format!("Failed to expand filesystem {:?}", mount_point);
        Err(EngineError::Engine(ErrorEnum::Error, err_msg))
    }
}

/// Create a filesystem on devnode.
pub fn create_fs(devnode: &Path) -> EngineResult<()> {
    if Command::new("mkfs.xfs")
           .arg("-f")
           .arg(&devnode)
           .status()?
           .success() {
        Ok(())
    } else {
        let err_msg = format!("Failed to create new filesystem at {:?}", devnode);
        Err(EngineError::Engine(ErrorEnum::Error, err_msg))
    }
}

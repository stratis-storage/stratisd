// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Manage the linear volume that stores metadata on pool levels 5-7.

use std::convert::From;
use std::fs::{create_dir, read_dir, remove_dir, remove_file, rename, OpenOptions};
use std::io::prelude::*;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use nix;
use nix::mount::{mount, umount, MsFlags};
use serde_json;

use devicemapper::{DmDevice, LinearDev, LinearDevTargetParams, TargetLine};

use crate::engine::{FilesystemUuid, Name, PoolUuid};
use crate::stratis::StratisResult;

use crate::engine::engine::DEV_PATH;
use crate::engine::strat_engine::cmd::create_fs;
use crate::engine::strat_engine::dm::get_dm;
use crate::engine::strat_engine::serde_structs::FilesystemSave;

use crate::engine::strat_engine::thinpool::filesystem::StratFilesystem;

// TODO: Monitor fs size and extend linear and fs if needed
// TODO: Document format of stuff on MDV in SWDD (currently ad-hoc)

const FILESYSTEM_DIR: &str = "filesystems";

#[derive(Debug)]
pub struct MetadataVol {
    dev: LinearDev,
    mount_pt: PathBuf,
}

/// A helper struct that borrows the MetadataVol and ensures that the MDV is
/// mounted as long as it is borrowed, and then unmounted.
#[derive(Debug)]
struct MountedMDV<'a> {
    mdv: &'a MetadataVol,
}

impl<'a> MountedMDV<'a> {
    /// Borrow the MDV and ensure it's mounted.
    fn mount(mdv: &MetadataVol) -> StratisResult<MountedMDV> {
        if let Err(err) = create_dir(&mdv.mount_pt) {
            if err.kind() != ErrorKind::AlreadyExists {
                return Err(From::from(err));
            }
        }

        match mount(
            Some(&mdv.dev.devnode()),
            &mdv.mount_pt,
            Some("xfs"),
            MsFlags::empty(),
            None as Option<&str>,
        ) {
            Err(nix::Error::Sys(nix::errno::Errno::EBUSY)) => {
                // The device is already mounted at the specified mount point
                Ok(())
            }
            Err(err) => Err(err),
            Ok(_) => Ok(()),
        }?;

        Ok(MountedMDV { mdv })
    }

    fn mount_pt(&self) -> &Path {
        &self.mdv.mount_pt
    }
}

impl<'a> Drop for MountedMDV<'a> {
    fn drop(&mut self) {
        if let Err(err) = umount(&self.mdv.mount_pt) {
            warn!("Could not unmount MDV: {}", err);
        } else if let Err(err) = remove_dir(&self.mdv.mount_pt) {
            warn!("Could not remove MDV mount point: {}", err);
        }
    }
}

impl MetadataVol {
    /// Initialize a new Metadata Volume.
    pub fn initialize(pool_uuid: PoolUuid, dev: LinearDev) -> StratisResult<MetadataVol> {
        create_fs(&dev.devnode(), pool_uuid)?;
        MetadataVol::setup(pool_uuid, dev)
    }

    /// Set up an existing Metadata Volume.
    pub fn setup(pool_uuid: PoolUuid, dev: LinearDev) -> StratisResult<MetadataVol> {
        let filename = format!(".mdv-{}", pool_uuid.to_simple_ref());
        let mount_pt: PathBuf = vec![DEV_PATH, &filename].iter().collect();

        let mdv = MetadataVol { dev, mount_pt };

        {
            let mount = MountedMDV::mount(&mdv)?;
            let filesystem_path = mount.mount_pt().join(FILESYSTEM_DIR);

            if let Err(err) = create_dir(&filesystem_path) {
                if err.kind() != ErrorKind::AlreadyExists {
                    return Err(From::from(err));
                }
            }

            let _ = remove_temp_files(&filesystem_path)?;
        }

        Ok(mdv)
    }

    /// Save info on a new filesystem to persistent storage, or update
    /// the existing info on a filesystem.
    // Write to a temp file and then rename to actual filename, to
    // ensure file contents are not truncated if operation is
    // interrupted.
    pub fn save_fs(
        &self,
        name: &Name,
        uuid: FilesystemUuid,
        fs: &StratFilesystem,
    ) -> StratisResult<()> {
        let data = serde_json::to_string(&fs.record(name, uuid))?;
        let path = self
            .mount_pt
            .join(FILESYSTEM_DIR)
            .join(uuid.to_simple_ref().to_string())
            .with_extension("json");

        let temp_path = path.clone().with_extension("temp");

        let _mount = MountedMDV::mount(self)?;

        // Braces to ensure f is closed before renaming
        {
            let mut f = OpenOptions::new()
                .write(true)
                .create(true)
                .open(&temp_path)?;
            f.write_all(data.as_bytes())?;

            // Try really hard to make sure it goes to disk
            f.sync_all()?;
        }

        rename(temp_path, path)?;

        Ok(())
    }

    /// Remove info on a filesystem from persistent storage.
    pub fn rm_fs(&self, fs_uuid: FilesystemUuid) -> StratisResult<()> {
        let fs_path = self
            .mount_pt
            .join(FILESYSTEM_DIR)
            .join(fs_uuid.to_simple_ref().to_string())
            .with_extension("json");

        let _mount = MountedMDV::mount(self)?;

        if let Err(err) = remove_file(fs_path) {
            if err.kind() != ErrorKind::NotFound {
                return Err(From::from(err));
            }
        }

        Ok(())
    }

    /// Get list of filesystems stored on the MDV.
    pub fn filesystems(&self) -> StratisResult<Vec<FilesystemSave>> {
        let mut filesystems = Vec::new();

        let mount = MountedMDV::mount(self)?;

        for dir_e in read_dir(mount.mount_pt().join(FILESYSTEM_DIR))? {
            let dir_e = dir_e?;

            if dir_e.path().ends_with(".temp") {
                continue;
            }

            let mut f = OpenOptions::new().read(true).open(&dir_e.path())?;
            let mut data = Vec::new();
            f.read_to_end(&mut data)?;

            filesystems.push(serde_json::from_slice(&data)?);
        }

        Ok(filesystems)
    }

    /// Tear down a Metadata Volume.
    pub fn teardown(&mut self) -> StratisResult<()> {
        self.dev.teardown(get_dm())?;

        Ok(())
    }

    /// Suspend the metadata volume DM devices
    pub fn suspend(&mut self) -> StratisResult<()> {
        self.dev.suspend(get_dm(), true)?;
        Ok(())
    }

    /// Resume the metadata volume DM devices
    pub fn resume(&mut self) -> StratisResult<()> {
        self.dev.resume(get_dm())?;
        Ok(())
    }

    /// Get a reference to the backing device
    pub fn device(&self) -> &LinearDev {
        &self.dev
    }

    /// Set the table of the backing device
    pub fn set_table(
        &mut self,
        table: Vec<TargetLine<LinearDevTargetParams>>,
    ) -> StratisResult<()> {
        self.dev.set_table(get_dm(), table)?;
        Ok(())
    }
}

/// Remove temp files from the designated directory.
/// Returns an error if the directory can not be read.
/// Persists if an individual directory entry can not be read due to an
/// intermittent IO error.
/// Returns the following summary values:
///  * the number of temp files found
///  * paths of those unremoved, if any
fn remove_temp_files(dir: &Path) -> StratisResult<(u64, Vec<PathBuf>)> {
    let mut found = 0;
    let mut failed = Vec::new();
    for path in read_dir(dir)?
        .filter_map(|e| e.ok()) // Just ignore entry on intermittent IO error
        .map(|e| e.path())
        .filter(|p| p.ends_with(".temp"))
    {
        found += 1;
        remove_file(&path).unwrap_or_else(|_| failed.push(path));
    }
    Ok((found, failed))
}

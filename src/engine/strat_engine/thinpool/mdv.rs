// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Manage the linear volume that stores metadata on pool levels 5-7.

use std::{
    convert::From,
    fs::{create_dir, create_dir_all, read_dir, remove_dir, remove_file, rename, OpenOptions},
    io::{prelude::*, ErrorKind},
    path::{Path, PathBuf},
};

use nix::mount::{mount, umount, MsFlags};
use retry::{delay::Fixed, retry_with_index};

use devicemapper::{DmDevice, DmOptions, LinearDev, LinearDevTargetParams, Sectors, TargetLine};

use crate::{
    engine::{
        strat_engine::{
            cmd::create_fs,
            dm::get_dm,
            serde_structs::FilesystemSave,
            thinpool::filesystem::{fs_usage, StratFilesystem},
        },
        types::{FilesystemUuid, Name, PoolUuid, StratisUuid},
    },
    stratis::StratisResult,
};

// TODO: Monitor fs size and extend linear and fs if needed
// TODO: Document format of stuff on MDV in SWDD (currently ad-hoc)

const RUN_DIR: &str = "/run/stratisd";
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
    fn mount(mdv: &MetadataVol) -> StratisResult<MountedMDV<'_>> {
        if let Err(err) = create_dir_all(&mdv.mount_pt) {
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
            Err(nix::Error::EBUSY) => {
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
        if let Err(e) = retry_with_index(Fixed::from_millis(100).take(2), |i| {
            trace!("MDV unmount attempt {}", i);
            umount(&self.mdv.mount_pt)
        }) {
            warn!("Unmounting MDV failed: {}", e);
            return;
        }
        if let Err(err) = remove_dir(&self.mdv.mount_pt) {
            warn!("Could not remove MDV mount point: {}", err);
        }
    }
}

impl MetadataVol {
    /// Minimum allocation size for a file is a block which will be 4k in this
    /// set up.
    const XFS_MIN_FILE_ALLOC_SIZE: Sectors = Sectors(8);

    /// Initialize a new Metadata Volume.
    pub fn initialize(pool_uuid: PoolUuid, dev: LinearDev) -> StratisResult<MetadataVol> {
        create_fs(&dev.devnode(), Some(StratisUuid::Pool(pool_uuid)), true)?;
        MetadataVol::setup(pool_uuid, dev)
    }

    /// Set up an existing Metadata Volume.
    pub fn setup(pool_uuid: PoolUuid, dev: LinearDev) -> StratisResult<MetadataVol> {
        let filename = format!(".mdv-{}", uuid_to_string!(pool_uuid));
        let mount_pt: PathBuf = vec![RUN_DIR, &filename].iter().collect();

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
            .join(uuid_to_string!(uuid))
            .with_extension("json");

        let temp_path = path.with_extension("temp");

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
            .join(uuid_to_string!(fs_uuid))
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
        self.dev.suspend(get_dm(), DmOptions::default())?;
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

    /// The maximum number of filesystems that can be recorded in the MDV.
    pub fn max_fs_limit(&self) -> StratisResult<u64> {
        let mounted = MountedMDV::mount(self)?;
        let (total_size, _) = fs_usage(mounted.mount_pt())?;
        Ok(total_size.sectors() / Self::XFS_MIN_FILE_ALLOC_SIZE)
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

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

use nix::{
    mount::{mount, umount, MsFlags},
    sys::stat::stat,
};
use retry::{delay::Fixed, retry_with_index};

use devicemapper::{DmDevice, DmOptions, LinearDev, LinearDevTargetParams, Sectors, TargetLine};

use crate::{
    engine::{
        strat_engine::{
            cmd::create_fs,
            dm::get_dm,
            ns::NS_TMPFS_LOCATION,
            serde_structs::FilesystemSave,
            thinpool::filesystem::{fs_usage, StratFilesystem},
        },
        types::{FilesystemUuid, Name, PoolUuid, StratisUuid},
    },
    stratis::{StratisError, StratisResult},
};

const FILESYSTEM_DIR: &str = "filesystems";

#[derive(Debug)]
pub struct MetadataVol {
    dev: LinearDev,
    mount_pt: PathBuf,
}

impl MetadataVol {
    /// Minimum allocation size for a file is a block which will be 4k in this
    /// set up.
    const XFS_MIN_FILE_ALLOC_SIZE: Sectors = Sectors(8);

    /// Initialize a new Metadata Volume.
    pub fn initialize(pool_uuid: PoolUuid, dev: LinearDev) -> StratisResult<MetadataVol> {
        create_fs(&dev.devnode(), Some(StratisUuid::Pool(pool_uuid)), false)?;
        MetadataVol::setup(pool_uuid, dev)
    }

    /// Set up an existing Metadata Volume.
    pub fn setup(pool_uuid: PoolUuid, dev: LinearDev) -> StratisResult<MetadataVol> {
        let filename = format!(".mdv-{}", uuid_to_string!(pool_uuid));
        let mount_pt: PathBuf = vec![NS_TMPFS_LOCATION, &filename].iter().collect();

        let mdv = MetadataVol { dev, mount_pt };

        {
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

            let filesystem_path = mdv.mount_pt.join(FILESYSTEM_DIR);

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

        for dir_e in read_dir(self.mount_pt.join(FILESYSTEM_DIR))? {
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
        if let Err(e) = retry_with_index(Fixed::from_millis(100).take(2), |i| {
            trace!("MDV unmount attempt {}", i);
            umount(&self.mount_pt)
        }) {
            return Err(match e {
                retry::Error::Internal(msg) => StratisError::Msg(msg),
                retry::Error::Operation { error, .. } => StratisError::Chained(
                    "Failed to unmount MDV".to_string(),
                    Box::new(StratisError::from(error)),
                ),
            });
        }

        if let Err(err) = remove_dir(&self.mount_pt) {
            warn!("Could not remove MDV mount point: {}", err);
        }

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
        let (total_size, _) = fs_usage(&self.mount_pt)?;
        Ok(total_size.sectors() / Self::XFS_MIN_FILE_ALLOC_SIZE)
    }
}

impl Drop for MetadataVol {
    fn drop(&mut self) {
        fn drop_failure(mount_pt: &PathBuf) -> StratisResult<()> {
            let mtpt_stat = match stat(mount_pt) {
                Ok(s) => s,
                Err(e) => match e {
                    nix::errno::Errno::ENOENT => return Ok(()),
                    e => return Err(StratisError::Nix(e)),
                },
            };
            let parent_stat = match stat(&mount_pt.join("..")) {
                Ok(s) => s,
                Err(e) => match e {
                    nix::errno::Errno::ENOENT => return Ok(()),
                    e => return Err(StratisError::Nix(e)),
                },
            };

            if mtpt_stat.st_dev != parent_stat.st_dev {
                if let Err(e) = retry_with_index(Fixed::from_millis(100).take(2), |i| {
                    trace!("MDV unmount attempt {}", i);
                    umount(mount_pt)
                }) {
                    Err(match e {
                        retry::Error::Internal(msg) => StratisError::Msg(msg),
                        retry::Error::Operation { error, .. } => StratisError::Chained(
                            "Failed to unmount MDV".to_string(),
                            Box::new(StratisError::from(error)),
                        ),
                    })
                } else {
                    Ok(())
                }
            } else {
                Ok(())
            }
        }

        if let Err(e) = drop_failure(&self.mount_pt) {
            warn!(
                "Failed to unmount MDV; some cleanup may not be able to be done: {}",
                e
            );
        }
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

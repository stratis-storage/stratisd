// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Manage the linear volume that stores metadata on pool levels 5-7.

use std::convert::From;
use std::error::Error;
use std::fs::{create_dir, OpenOptions, read_dir, remove_file, rename};
use std::io::ErrorKind;
use std::io::prelude::*;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

use devicemapper::{LinearDev, DM, Segment};
use serde_json;
use nix::unistd::fsync;

use super::super::engine::HasUuid;
use super::super::errors::EngineResult;
use super::super::types::{FilesystemUuid, PoolUuid};

use super::engine::DEV_PATH;
use super::filesystem::{create_fs, mount_fs, unmount_fs, StratFilesystem};
use super::serde_structs::{FilesystemSave, Recordable};

// TODO: Monitor fs size and extend linear and fs if needed
// TODO: Document format of stuff on MDV in SWDD (currently ad-hoc)

const FILESYSTEM_DIR: &'static str = "filesystems";

#[derive(Debug)]
pub struct MetadataVol {
    dev: LinearDev,
    mount_pt: PathBuf,
}

impl MetadataVol {
    /// Initialize a new Metadata Volume.
    pub fn initialize(pool_uuid: &PoolUuid, dev: LinearDev) -> EngineResult<MetadataVol> {
        try!(create_fs(try!(dev.devnode()).as_path()));
        MetadataVol::setup(pool_uuid, dev)
    }

    /// Set up an existing Metadata Volume.
    pub fn setup(pool_uuid: &PoolUuid, dev: LinearDev) -> EngineResult<MetadataVol> {
        let filename = format!(".mdv-{}", pool_uuid.simple());
        let mount_pt: PathBuf = vec![DEV_PATH, &filename].iter().collect();

        if let Err(err) = create_dir(&mount_pt) {
            if err.kind() != ErrorKind::AlreadyExists {
                return Err(From::from(err));
            }
        }

        try!(mount_fs(&try!(dev.devnode()), &mount_pt));

        if let Err(err) = create_dir(&mount_pt.join(FILESYSTEM_DIR)) {
            if err.kind() != ErrorKind::AlreadyExists {
                return Err(From::from(err));
            }
        }

        Ok(MetadataVol { dev, mount_pt })
    }

    /// Save info on a new filesystem to persistent storage, or update
    /// the existing info on a filesystem.
    // Write to a temp file and then rename to actual filename, to
    // ensure file contents are not truncated if operation is
    // interrupted.
    pub fn save_fs(&self, fs: &StratFilesystem) -> EngineResult<()> {
        let data = try!(serde_json::to_string(&try!(fs.record())));
        let path = self.mount_pt
            .join(FILESYSTEM_DIR)
            .join(fs.uuid().simple().to_string())
            .with_extension("json");

        let temp_path = path.clone().with_extension("temp");

        // Braces to ensure f is closed before renaming
        {
            let mut f = try!(OpenOptions::new()
                                 .write(true)
                                 .create(true)
                                 .open(&temp_path));
            try!(f.write_all(data.as_bytes()));

            // Try really hard to make sure it goes to disk
            try!(f.flush());
            try!(fsync(f.as_raw_fd()));
        }

        try!(rename(temp_path, path));

        Ok(())
    }

    /// Remove info on a filesystem from persistent storage.
    pub fn rm_fs(&self, fs_uuid: &FilesystemUuid) -> EngineResult<()> {
        let fs_path = self.mount_pt
            .join(FILESYSTEM_DIR)
            .join(fs_uuid.simple().to_string())
            .with_extension("json");
        if let Err(err) = remove_file(fs_path) {
            if err.kind() != ErrorKind::NotFound {
                return Err(From::from(err));
            }
        }

        Ok(())
    }

    /// Check the current state of the MDV.
    pub fn check(&self) -> EngineResult<()> {
        for dir_e in try!(read_dir(self.mount_pt.join(FILESYSTEM_DIR))) {
            let dir_e = try!(dir_e);

            // Clean up any lingering .temp files. These should only
            // exist if there was a crash during save_fs().
            if dir_e.path().ends_with(".temp") {
                match remove_file(dir_e.path()) {
                    Err(err) => {
                        debug!("could not remove file {:?}: {}",
                               dir_e.path(),
                               err.description())
                    }
                    Ok(_) => debug!("Cleaning up temp file {:?}", dir_e.path()),
                }
            }
        }

        Ok(())
    }

    /// Get list of filesystems stored on the MDV.
    pub fn filesystems(&self) -> EngineResult<Vec<FilesystemSave>> {
        let mut filesystems = Vec::new();

        for dir_e in try!(read_dir(self.mount_pt.join(FILESYSTEM_DIR))) {
            let dir_e = try!(dir_e);

            if dir_e.path().ends_with(".temp") {
                continue;
            }

            let mut f = try!(OpenOptions::new().read(true).open(&dir_e.path()));
            let mut data = Vec::new();
            try!(f.read_to_end(&mut data));

            filesystems.push(try!(serde_json::from_slice(&data)));
        }

        Ok(filesystems)
    }

    /// Return the segments used.
    pub fn segments(&self) -> &[Segment] {
        self.dev.segments()
    }

    /// Tear down a Metadata Volume.
    pub fn teardown(self, dm: &DM) -> EngineResult<()> {
        let v: Vec<&str> = Vec::new();
        try!(unmount_fs(&self.mount_pt, &v));
        try!(self.dev.teardown(dm));

        Ok(())
    }
}

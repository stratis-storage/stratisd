// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::str;

use super::super::errors::EngineResult;

use engine::Pool;

pub const DEV_PATH: &str = "/dev/stratis";

/// Set up directories and symlinks under /dev/stratis based on current
/// config. Clear out any directory or file that doesn't correspond to a pool
/// or filesystem.
// Don't just remove and recreate everything in case there are processes
// (e.g. user shells) with the current working directory within the tree.
pub fn setup_devlinks<'a, I: Iterator<Item = &'a Pool>>(pools: I) -> EngineResult<()> {
    if let Err(err) = fs::create_dir(DEV_PATH) {
        if err.kind() != ErrorKind::AlreadyExists {
            return Err(From::from(err));
        }
    }

    let mut existing_dirs = fs::read_dir(DEV_PATH)?
        .map(|dir_e| dir_e.and_then(|d| Ok(d.file_name().into_string().expect("Unix is utf-8"))))
        .collect::<Result<HashSet<_>, _>>()?;

    for pool in pools {
        if !existing_dirs.remove(pool.name()) {
            pool_added(pool.name())?;
        }

        let pool_path: PathBuf = vec![DEV_PATH, pool.name()].iter().collect();

        let mut existing_files = fs::read_dir(pool_path)?
            .map(|dir_e| {
                     dir_e.and_then(|d| Ok(d.file_name().into_string().expect("Unix is utf-8")))
                 })
            .collect::<Result<HashSet<_>, _>>()?;

        for fs in pool.filesystems() {
            filesystem_added(pool.name(), fs.name(), &fs.devnode())?;
            existing_files.remove(fs.name());
        }

        for leftover in existing_files {
            filesystem_removed(pool.name(), &leftover)?;
        }
    }

    for leftover in existing_dirs {
        pool_removed(str::from_utf8(leftover.as_bytes()).expect("is valid utf8"))?
    }

    Ok(())
}

/// Create a directory when a pool is added.
pub fn pool_added(pool: &str) -> EngineResult<()> {
    let p: PathBuf = vec![DEV_PATH, pool].iter().collect();
    fs::create_dir(&p)?;
    Ok(())
}

/// Remove the directory and its contents when the pool is removed.
pub fn pool_removed(pool: &str) -> EngineResult<()> {
    let p: PathBuf = vec![DEV_PATH, pool].iter().collect();
    fs::remove_dir_all(&p)?;
    Ok(())
}

/// Rename the directory to match the pool's new name.
pub fn pool_renamed(old_name: &str, new_name: &str) -> EngineResult<()> {
    let old: PathBuf = vec![DEV_PATH, old_name].iter().collect();
    let new: PathBuf = vec![DEV_PATH, new_name].iter().collect();
    fs::rename(&old, &new)?;
    Ok(())
}

/// Create a symlink to the new filesystem's block device within its pool's
/// directory.
pub fn filesystem_added(pool_name: &str, fs_name: &str, devnode: &Path) -> EngineResult<()> {
    let p: PathBuf = vec![DEV_PATH, pool_name, fs_name].iter().collect();

    // Remove existing and recreate to ensure it points to the correct devnode
    let _ = fs::remove_file(&p);
    symlink(devnode, &p)?;
    Ok(())
}

/// Remove the symlink when the filesystem is destroyed.
pub fn filesystem_removed(pool_name: &str, fs_name: &str) -> EngineResult<()> {
    let p: PathBuf = vec![DEV_PATH, pool_name, fs_name].iter().collect();
    fs::remove_file(&p)?;
    Ok(())
}

/// Rename the symlink to track the filesystem's new name.
pub fn filesystem_renamed(pool_name: &str, old_name: &str, new_name: &str) -> EngineResult<()> {
    let old: PathBuf = vec![DEV_PATH, pool_name, old_name].iter().collect();
    let new: PathBuf = vec![DEV_PATH, pool_name, new_name].iter().collect();
    fs::rename(&old, &new)?;
    Ok(())
}

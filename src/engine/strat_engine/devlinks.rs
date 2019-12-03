// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::HashSet,
    fs,
    io::ErrorKind,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    str,
};

use crate::{
    engine::{
        engine::{Pool, DEV_PATH},
        types::{Name, PoolUuid},
    },
    stratis::StratisResult,
};

/// Set up the root Stratis directory, where dev links as well as temporary
/// MDV mounts will be created. This must occur before any pools are setup.
pub fn setup_dev_path() -> StratisResult<()> {
    if let Err(err) = fs::create_dir(DEV_PATH) {
        if err.kind() != ErrorKind::AlreadyExists {
            return Err(From::from(err));
        }
    }

    Ok(())
}

/// Setup the pool directory and the symlinks in /stratis for the specified pool and filesystems
/// it contains.
// Don't just remove and recreate everything in case there are processes
// (e.g. user shells) with the current working directory within the tree.
pub fn setup_pool_devlinks(pool_name: &str, pool: &dyn Pool) {
    if let Err(err) = || -> StratisResult<()> {
        let pool_path = pool_directory(pool_name);

        if !pool_path.exists() {
            pool_added(pool_name);
        }

        let mut existing_files = fs::read_dir(pool_path)?
            .map(|dir_e| {
                dir_e.and_then(|d| Ok(d.file_name().into_string().expect("Unix is utf-8")))
            })
            .collect::<Result<HashSet<_>, _>>()?;

        for (fs_name, _, fs) in pool.filesystems() {
            filesystem_added(pool_name, &fs_name, &fs.devnode());
            existing_files.remove(&fs_name.to_owned());
        }

        for leftover in existing_files {
            filesystem_removed(pool_name, &leftover);
        }

        Ok(())
    }() {
        warn!(
            "setup_pool_devlinks failed for /stratis/{}, reason {:?}",
            pool_name, err
        );
    };
}

/// Clean up directories and symlinks under /stratis based on current
/// config. Clear out any directory or file that doesn't correspond to a pool.
// Don't just remove everything in case there are processes
// (e.g. user shells) with the current working directory within the tree.
pub fn cleanup_devlinks<'a, I: Iterator<Item = &'a (Name, PoolUuid, &'a dyn Pool)>>(pools: I) {
    if let Err(err) = || -> StratisResult<()> {
        let mut existing_dirs = fs::read_dir(DEV_PATH)?
            .map(|dir_e| {
                dir_e.and_then(|d| Ok(d.file_name().into_string().expect("Unix is utf-8")))
            })
            .collect::<Result<HashSet<_>, _>>()?;

        for &(ref pool_name, _, _) in pools {
            existing_dirs.remove(&pool_name.to_owned());
        }

        for leftover in existing_dirs {
            pool_removed(&Name::new(leftover));
        }

        Ok(())
    }() {
        warn!("cleanup_devlinks failed, reason {:?}", err);
    }
}

/// Create a directory when a pool is added.
pub fn pool_added(pool: &str) {
    let p = pool_directory(pool);
    if let Err(e) = fs::create_dir(&p) {
        warn!("unable to create pool directory {:?}, reason {:?}", p, e);
    }
}

/// Remove the directory and its contents when the pool is removed.
pub fn pool_removed(pool: &str) {
    let p = pool_directory(pool);
    if let Err(e) = fs::remove_dir_all(&p) {
        warn!("unable to remove pool directory {:?}, reason {:?}", p, e);
    }
}

/// Rename the directory to match the pool's new name.
pub fn pool_renamed(old_name: &str, new_name: &str) {
    let old = pool_directory(old_name);
    let new = pool_directory(new_name);
    if let Err(e) = fs::rename(&old, &new) {
        warn!(
            "unable to rename pool directory old {:?}, new {:?}, reason {:?}",
            old, new, e
        );
    }
}

/// Create a symlink to the new filesystem's block device within its pool's
/// directory.
pub fn filesystem_added(pool_name: &str, fs_name: &str, devnode: &Path) {
    let p = filesystem_mount_path(pool_name, fs_name);

    // Remove existing and recreate to ensure it points to the correct devnode
    let _ = fs::remove_file(&p);
    if let Err(e) = symlink(devnode, &p) {
        warn!(
            "unable to create symlink for {:?} -> {:?}, reason {:?}",
            devnode, p, e
        );
    }
}

/// Remove the symlink when the filesystem is destroyed.
pub fn filesystem_removed(pool_name: &str, fs_name: &str) {
    let p = filesystem_mount_path(pool_name, fs_name);
    if let Err(e) = fs::remove_file(&p) {
        warn!(
            "unable to remove symlink for filesystem {:?}, reason {:?}",
            p, e
        );
    }
}

/// Rename the symlink to track the filesystem's new name.
pub fn filesystem_renamed(pool_name: &str, old_name: &str, new_name: &str) {
    let old = filesystem_mount_path(pool_name, old_name);
    let new = filesystem_mount_path(pool_name, new_name);
    if let Err(e) = fs::rename(&old, &new) {
        warn!(
            "unable to rename filesystem symlink for {:?} -> {:?}, reason {:?}",
            old, new, e
        );
    }
}

/// Given a pool name, synthesize a pool directory name for storing filesystem
/// mount paths.
fn pool_directory<T: AsRef<str>>(pool_name: T) -> PathBuf {
    vec![DEV_PATH, pool_name.as_ref()].iter().collect()
}

/// Given a pool name and a filesystem name, return the path it should be
/// available as a device for mounting.
pub fn filesystem_mount_path<T: AsRef<str>>(pool_name: T, fs_name: T) -> PathBuf {
    vec![DEV_PATH, pool_name.as_ref(), fs_name.as_ref()]
        .iter()
        .collect()
}

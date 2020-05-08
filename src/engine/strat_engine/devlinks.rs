// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[cfg(test)]
use std::{collections::HashSet, io::ErrorKind};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    str,
};

#[cfg(test)]
use crate::engine::{
    strat_engine::pool::StratPool,
    types::{Name, PoolUuid},
};
use crate::{
    engine::engine::DEV_PATH,
    stratis::{ErrorEnum, StratisError, StratisResult},
};

const UEVENT_PATH: &str = "/sys/class/block";
const UEVENT_CHANGE_EVENT: &[u8] = b"change";

pub fn pool_renamed(pool_name: &str) -> StratisResult<()> {
    let pool_dir: PathBuf = [DEV_PATH, pool_name].iter().collect();

    for file_result in fs::read_dir(&pool_dir)? {
        let file = file_result?;
        let file_path = file.path();
        let file_name = file_path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| {
                StratisError::Engine(
                    ErrorEnum::Invalid,
                    format!(
                        "Failure while attempting to generate uevents for all \
                        of the devices contained in directory {} to make /dev/stratis \
                        symlinks consistent with internal pool state.",
                        pool_dir.display(),
                    ),
                )
            })?;
        fs_renamed(pool_name, file_name)?
    }

    Ok(())
}

pub fn fs_renamed(pool_name: &str, fs_name: &str) -> StratisResult<()> {
    let path: PathBuf = [DEV_PATH, pool_name, fs_name].iter().collect();

    let dm_path = path.canonicalize()?;
    let file_name = dm_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| {
            StratisError::Engine(
                ErrorEnum::NotFound,
                format!(
                    "Failed to generate a change uevent for {} to \
                make /dev/stratis symlinks consistent with internal pool state",
                    dm_path.display()
                ),
            )
        })?;

    let uevent_path: PathBuf = [UEVENT_PATH, file_name, "uevent"].iter().collect();

    let mut uevent_file = OpenOptions::new().write(true).open(&uevent_path)?;
    uevent_file.write_all(UEVENT_CHANGE_EVENT)?;

    Ok(())
}

/// Set up the root Stratis directory, where dev links as well as temporary
/// MDV mounts will be created. This must occur before any pools are setup.
#[cfg(test)]
pub fn setup_dev_path() -> StratisResult<()> {
    if let Err(err) = fs::create_dir(DEV_PATH) {
        if err.kind() != ErrorKind::AlreadyExists {
            return Err(From::from(err));
        }
    }

    Ok(())
}

/// Clean up directories and symlinks under /stratis based on current
/// config. Clear out any directory or file that doesn't correspond to a pool.
// Don't just remove everything in case there are processes
// (e.g. user shells) with the current working directory within the tree.
#[cfg(test)]
pub fn cleanup_devlinks<'a, I: Iterator<Item = (&'a Name, &'a PoolUuid, &'a StratPool)>>(pools: I) {
    if let Err(err) = || -> StratisResult<()> {
        let mut existing_dirs = fs::read_dir(DEV_PATH)?
            .map(|dir_e| dir_e.map(|d| d.file_name().into_string().expect("Unix is utf-8")))
            .collect::<Result<HashSet<_>, _>>()?;

        for (pool_name, _, _) in pools {
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
#[cfg(test)]
pub fn pool_added(pool: &str) {
    let p = pool_directory(pool);
    if let Err(e) = fs::create_dir(&p) {
        warn!("unable to create pool directory {:?}, reason {:?}", p, e);
    }
}

/// Remove the directory and its contents when the pool is removed.
#[cfg(test)]
pub fn pool_removed(pool: &str) {
    let p = pool_directory(pool);
    if let Err(e) = fs::remove_dir_all(&p) {
        warn!("unable to remove pool directory {:?}, reason {:?}", p, e);
    }
}

/// Given a pool name, synthesize a pool directory name for storing filesystem
/// mount paths.
#[cfg(test)]
fn pool_directory<T: AsRef<str>>(pool_name: T) -> PathBuf {
    vec![DEV_PATH, pool_name.as_ref()].iter().collect()
}

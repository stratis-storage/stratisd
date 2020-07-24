// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    str,
};

use crate::{
    engine::engine::DEV_PATH,
    stratis::{ErrorEnum, StratisError, StratisResult},
};

const UEVENT_PATH: &str = "/sys/class/block";
const UEVENT_CHANGE_EVENT: &[u8] = b"change";

/// Given a pool name and a filesystem name, return the path it should be
/// available as a device for mounting.
pub fn filesystem_mount_path<T: AsRef<str>>(pool_name: T, fs_name: T) -> PathBuf {
    vec![DEV_PATH, pool_name.as_ref(), fs_name.as_ref()]
        .iter()
        .collect()
}

/// Triggers a udev event for every filesystem in the pool to cause a rename for
/// the pool directory by moving all filesystem symlinks to the new pool directory.
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
                    ErrorEnum::Error,
                    format!(
                        "Failure while attempting to generate uevents for all \
                        of the devices contained in directory {} to make /dev/stratis \
                        symlinks consistent with internal pool state.",
                        pool_dir.display(),
                    ),
                )
            })?;
        filesystem_renamed(pool_name, file_name)?;
    }

    Ok(())
}

/// Trigger a udev event to pick up the new name of the filesystem as registered
/// with stratisd and rename the symlink.
pub fn filesystem_renamed(pool_name: &str, fs_name: &str) -> StratisResult<()> {
    let path = filesystem_mount_path(pool_name, fs_name);

    let dm_path = path.canonicalize()?;
    let file_name = dm_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| {
            StratisError::Engine(
                ErrorEnum::Error,
                format!(
                    "Failed to generate a change uevent for {} to \
                    make /dev/stratis symlinks consistent with internal filesystem \
                    state",
                    dm_path.display()
                ),
            )
        })?;

    let uevent_path: PathBuf = [UEVENT_PATH, file_name, "uevent"].iter().collect();

    let mut uevent_file = OpenOptions::new().write(true).open(&uevent_path)?;
    uevent_file.write_all(UEVENT_CHANGE_EVENT)?;

    Ok(())
}

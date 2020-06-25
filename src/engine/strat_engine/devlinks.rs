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

pub fn pool_renamed(pool_name: &str, _: &str) {
    fn trigger_udev(pool_name: &str) -> StratisResult<()> {
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
            filesystem_renamed(pool_name, file_name, file_name);
        }

        Ok(())
    }

    if let Err(e) = trigger_udev(pool_name) {
        warn!(
            "Synthetic udev events were not able to be triggered: {}. Migration of \
            filesystem links associated with renamed pool {} failed",
            e, pool_name,
        );
    }
}

pub fn filesystem_renamed(pool_name: &str, fs_name: &str, _: &str) {
    fn trigger_udev(pool_name: &str, fs_name: &str) -> StratisResult<()> {
        let path = filesystem_mount_path(pool_name, fs_name);

        let dm_path = path.canonicalize()?;
        let file_name = dm_path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| {
                StratisError::Engine(
                    ErrorEnum::NotFound,
                    format!(
                        "Failed to generate a change uevent for {} to \
                        make /dev/stratis symlinks consistent with internal pool \
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

    if let Err(e) = trigger_udev(pool_name, fs_name) {
        warn!(
            "Synthetic udev event was not able to be triggered: {}. Rename of \
            filesystem link {} failed",
            e, fs_name,
        );
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Handles invoking external binaries.
// This module assumes that, for a given machine, there is only one place
// where the desired executable might be installed. It expects the engine
// to identify that place at its initialization by invoking verify_binaries(),
// and to exit immediately if verify_binaries() return an error. If this
// protocol is followed then when any command is executed the unique absolute
// path of the binary for this machine will already have been identified.
// However stratisd may run for a while and it is possible for the binary
// to be caused to be uninstalled while stratisd is being run. Therefore,
// the existence of the file is checked before the command is invoked, and
// an explicit error is returned if the executable can not be found.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
};

use uuid::Uuid;

use crate::engine::errors::{Error, ErrorKind};

const BINARIES_PATHS: [&str; 4] = ["/usr/sbin", "/sbin", "/usr/bin", "/bin"];

/// Find the binary with the given name by looking in likely locations.
/// Return None if no binary was found.
/// Search an explicit list of directories rather than the user's PATH
/// environment variable. stratisd may be running when there is no PATH
/// variable set.
fn find_binary(name: &str) -> Option<PathBuf> {
    BINARIES_PATHS
        .iter()
        .map(|pre| [pre, name].iter().collect::<PathBuf>())
        .find(|path| path.exists())
}

// These are the external binaries that stratisd relies on.
// Any change in this list requires a corresponding change to BINARIES,
// and vice-versa.
const MKFS_XFS: &str = "mkfs.xfs";
const THIN_CHECK: &str = "thin_check";
const THIN_REPAIR: &str = "thin_repair";
const UDEVADM: &str = "udevadm";
const XFS_DB: &str = "xfs_db";
const XFS_GROWFS: &str = "xfs_growfs";

lazy_static! {
    static ref BINARIES: HashMap<String, Option<PathBuf>> = [
        (MKFS_XFS.to_string(), find_binary(MKFS_XFS)),
        (THIN_CHECK.to_string(), find_binary(THIN_CHECK)),
        (THIN_REPAIR.to_string(), find_binary(THIN_REPAIR)),
        (UDEVADM.to_string(), find_binary(UDEVADM)),
        (XFS_DB.to_string(), find_binary(XFS_DB)),
        (XFS_GROWFS.to_string(), find_binary(XFS_GROWFS)),
    ]
    .iter()
    .cloned()
    .collect();
}

/// Verify that all binaries that the engine might invoke are available at some
/// path. Return an error if any are missing. Required to be called on engine
/// initialization.
pub fn verify_binaries() -> Result<(), Error> {
    let missing: Vec<String> = BINARIES
        .iter()
        .filter(|&(_, ref path)| path.is_none())
        .map(|(name, _)| name.to_owned())
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::BinariesNotFound {
            names: missing,
            locations: BINARIES_PATHS.iter().map(|path| path.to_string()).collect(),
        }))
    }
}

/// Invoke the specified command. Return an error if invoking the command
/// fails or if the command itself fails.
fn execute_cmd(cmd: &mut Command) -> Result<(), Error> {
    match cmd.output() {
        Err(err) => Err(Error::new(ErrorKind::CommandExecutionFailure {
            cmd: format!("{:?}", cmd),
        })
        .set_constituent(Box::new(err))),
        Ok(result) => {
            if result.status.success() {
                Ok(())
            } else {
                Err(Error::new(ErrorKind::CommandFailure {
                    cmd: format!("{:?}", cmd),
                    output: result,
                }))
            }
        }
    }
}

/// Get an absolute path for the executable with the given name.
/// Precondition: verify_binaries() has already been invoked.
fn get_executable(name: &str) -> &Path {
    BINARIES
        .get(name)
        .expect("name arguments are all constants defined with BINARIES, lookup can not fail")
        .as_ref()
        .expect("verify_binaries() was previously called and returned no error")
}

/// Create a filesystem on devnode.
pub fn create_fs(devnode: &Path, uuid: Uuid) -> Result<(), Error> {
    execute_cmd(
        Command::new(get_executable(MKFS_XFS).as_os_str())
            .arg("-f")
            .arg("-q")
            .arg(&devnode)
            .arg("-m")
            .arg(format!("uuid={}", uuid)),
    )
}

/// Use the xfs_growfs command to expand a filesystem mounted at the given
/// mount point.
pub fn xfs_growfs(mount_point: &Path) -> Result<(), Error> {
    execute_cmd(
        Command::new(get_executable(XFS_GROWFS).as_os_str())
            .arg(mount_point)
            .arg("-d"),
    )
}

/// Set a new UUID for filesystem on the devnode.
pub fn set_uuid(devnode: &Path, uuid: Uuid) -> Result<(), Error> {
    execute_cmd(
        Command::new(get_executable(XFS_DB).as_os_str())
            .arg("-x")
            .arg(format!("-c uuid {}", uuid))
            .arg(&devnode),
    )
}

/// Call thin_check on a thinpool
pub fn thin_check(devnode: &Path) -> Result<(), Error> {
    execute_cmd(
        Command::new(get_executable(THIN_CHECK).as_os_str())
            .arg("-q")
            .arg(devnode),
    )
}

/// Call thin_repair on a thinpool
pub fn thin_repair(meta_dev: &Path, new_meta_dev: &Path) -> Result<(), Error> {
    execute_cmd(
        Command::new(get_executable(THIN_REPAIR).as_os_str())
            .arg("-i")
            .arg(meta_dev)
            .arg("-o")
            .arg(new_meta_dev),
    )
}

/// Call udevadm settle
pub fn udev_settle() -> Result<(), Error> {
    execute_cmd(Command::new(get_executable(UDEVADM).as_os_str()).arg("settle"))
}

#[cfg(test)]
pub fn create_ext3_fs(devnode: &Path) -> Result<(), Error> {
    execute_cmd(Command::new("wipefs").arg("-a").arg(&devnode))?;
    execute_cmd(Command::new("mkfs.ext3").arg(&devnode))
}

#[cfg(test)]
#[allow(dead_code)]
pub fn xfs_repair(devnode: &Path) -> Result<(), Error> {
    execute_cmd(Command::new("xfs_repair").arg("-n").arg(&devnode))
}

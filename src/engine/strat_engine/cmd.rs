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

use serde_json::Value;

use crate::{
    engine::types::{FilesystemUuid, StratisUuid},
    stratis::{StratisError, StratisResult},
};

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
const CLEVIS: &str = "clevis";
const CLEVIS_BIND: &str = "clevis-luks-bind";
const CLEVIS_UNBIND: &str = "clevis-luks-unbind";
const CLEVIS_UNLOCK: &str = "clevis-luks-unlock";

lazy_static! {
    static ref BINARIES: HashMap<String, Option<PathBuf>> = [
        (MKFS_XFS.to_string(), find_binary(MKFS_XFS)),
        (THIN_CHECK.to_string(), find_binary(THIN_CHECK)),
        (THIN_REPAIR.to_string(), find_binary(THIN_REPAIR)),
        (UDEVADM.to_string(), find_binary(UDEVADM)),
        (XFS_DB.to_string(), find_binary(XFS_DB)),
        (XFS_GROWFS.to_string(), find_binary(XFS_GROWFS)),
        (CLEVIS.to_string(), find_binary(CLEVIS)),
        (CLEVIS_BIND.to_string(), find_binary(CLEVIS_BIND)),
        (CLEVIS_UNBIND.to_string(), find_binary(CLEVIS_UNBIND)),
        (CLEVIS_UNLOCK.to_string(), find_binary(CLEVIS_UNLOCK)),
    ]
    .iter()
    .cloned()
    .collect();
}

/// Verify that all binaries that the engine might invoke are available at some
/// path. Return an error if any are missing. Required to be called on engine
/// initialization.
pub fn verify_binaries() -> StratisResult<()> {
    match BINARIES.iter().find(|&(_, path)| path.is_none()) {
        None => Ok(()),
        Some((name, _)) => Err(StratisError::Error(format!(
            "Unable to find executable \"{}\" in any of {}",
            name,
            BINARIES_PATHS
                .iter()
                .map(|p| format!("\"{}\"", p))
                .collect::<Vec<_>>()
                .join(", "),
        ))),
    }
}

/// Invoke the specified command. Return an error if invoking the command
/// fails or if the command itself fails.
fn execute_cmd(cmd: &mut Command) -> StratisResult<()> {
    match cmd.output() {
        Err(err) => Err(StratisError::Error(format!(
            "Failed to execute command {:?}, err: {:?}",
            cmd, err
        ))),
        Ok(result) => {
            if result.status.success() {
                Ok(())
            } else {
                let exit_reason = result
                    .status
                    .code()
                    .map_or(String::from("process terminated by signal"), |ec| {
                        ec.to_string()
                    });
                let std_out_txt = String::from_utf8_lossy(&result.stdout);
                let std_err_txt = String::from_utf8_lossy(&result.stderr);
                let err_msg = format!(
                    "Command failed: cmd: {:?}, exit reason: {} stdout: {} stderr: {}",
                    cmd, exit_reason, std_out_txt, std_err_txt
                );
                Err(StratisError::Error(err_msg))
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

/// Create a filesystem on devnode. If uuid specified, set the UUID of the
/// filesystem on creation. `noalign` should be `true` when creating small metadata filesystems
/// like the MDV.
pub fn create_fs(devnode: &Path, uuid: Option<StratisUuid>, noalign: bool) -> StratisResult<()> {
    let mut command = Command::new(get_executable(MKFS_XFS).as_os_str());
    command.arg("-f");
    command.arg("-q");
    command.arg(devnode);

    if let Some(uuid) = uuid {
        command.arg("-m");
        command.arg(format!("uuid={}", uuid));
    }
    if noalign {
        command.arg("-d");
        command.arg("noalign");
    }
    execute_cmd(&mut command)
}

/// Use the xfs_growfs command to expand a filesystem mounted at the given
/// mount point.
pub fn xfs_growfs(mount_point: &Path) -> StratisResult<()> {
    execute_cmd(
        Command::new(get_executable(XFS_GROWFS).as_os_str())
            .arg(mount_point)
            .arg("-d"),
    )
}

/// Set a new UUID for filesystem on the devnode.
pub fn set_uuid(devnode: &Path, uuid: FilesystemUuid) -> StratisResult<()> {
    execute_cmd(
        Command::new(get_executable(XFS_DB).as_os_str())
            .arg("-x")
            .arg(format!("-c uuid {}", uuid))
            .arg(devnode),
    )
}

/// Call thin_check on a thinpool
pub fn thin_check(devnode: &Path) -> StratisResult<()> {
    execute_cmd(
        Command::new(get_executable(THIN_CHECK).as_os_str())
            .arg("-q")
            .arg(devnode),
    )
}

/// Call thin_repair on a thinpool
pub fn thin_repair(meta_dev: &Path, new_meta_dev: &Path) -> StratisResult<()> {
    execute_cmd(
        Command::new(get_executable(THIN_REPAIR).as_os_str())
            .arg("-i")
            .arg(meta_dev)
            .arg("-o")
            .arg(new_meta_dev),
    )
}

/// Call udevadm settle
pub fn udev_settle() -> StratisResult<()> {
    execute_cmd(Command::new(get_executable(UDEVADM).as_os_str()).arg("settle"))
}

/// Bind a LUKS device using clevis.
pub fn clevis_luks_bind(
    dev_path: &Path,
    keyfile_path: &Path,
    pin: &str,
    json: &Value,
    yes: bool,
) -> StratisResult<()> {
    let mut cmd = Command::new(CLEVIS);

    cmd.arg("luks").arg("bind");

    if yes {
        cmd.arg("-y");
    };

    cmd.arg("-d")
        .arg(dev_path.display().to_string())
        .arg("-k")
        .arg(keyfile_path)
        .arg(pin)
        .arg(json.to_string());

    execute_cmd(&mut cmd)
}

/// Unbind a LUKS device using clevis.
pub fn clevis_luks_unbind(dev_path: &Path, keyslot: libc::c_uint) -> StratisResult<()> {
    execute_cmd(
        Command::new(CLEVIS)
            .arg("luks")
            .arg("unbind")
            .arg("-d")
            .arg(dev_path.display().to_string())
            .arg("-s")
            .arg(keyslot.to_string())
            .arg("-f"),
    )
}

/// Unlock a device using the clevis CLI.
pub fn clevis_luks_unlock(dev_path: &Path, dm_name: &str) -> StratisResult<()> {
    execute_cmd(
        Command::new(CLEVIS)
            .arg("luks")
            .arg("unlock")
            .arg("-d")
            .arg(dev_path.display().to_string())
            .arg("-n")
            .arg(dm_name),
    )
}

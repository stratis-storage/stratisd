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
    cmp::min,
    collections::HashMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use libc::c_uint;
use libcryptsetup_rs::SafeMemHandle;
use serde_json::Value;

use devicemapper::{MetaBlocks, Sectors};

use crate::{
    engine::{
        engine::MAX_STRATIS_PASS_SIZE,
        types::{FilesystemUuid, SizedKeyMemory, StratisUuid},
    },
    stratis::{StratisError, StratisResult},
};

// The maximum allowable size of the thinpool metadata device
const MAX_META_SIZE: MetaBlocks = MetaBlocks(255 * ((1 << 14) - 64));

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
const THIN_METADATA_SIZE: &str = "thin_metadata_size";
const XFS_DB: &str = "xfs_db";
const XFS_GROWFS: &str = "xfs_growfs";
const CLEVIS: &str = "clevis";
const CLEVIS_LIB: &str = "clevis-luks-common-functions";
const CLEVIS_BIND: &str = "clevis-luks-bind";
const CLEVIS_UNBIND: &str = "clevis-luks-unbind";
const CLEVIS_UNLOCK: &str = "clevis-luks-unlock";
const CLEVIS_DECRYPT: &str = "clevis-decrypt";
const CLEVIS_DECRYPT_TANG: &str = "clevis-decrypt-tang";
const CLEVIS_DECRYPT_TPM2: &str = "clevis-decrypt-tpm2";
const CLEVIS_ENCRYPT_TANG: &str = "clevis-encrypt-tang";
const CLEVIS_ENCRYPT_TPM2: &str = "clevis-encrypt-tpm2";
const CLEVIS_REGEN: &str = "clevis-luks-regen";
const JOSE: &str = "jose";
const JQ: &str = "jq";
const CRYPTSETUP: &str = "cryptsetup";
const CURL: &str = "curl";
const TPM2_CREATEPRIMARY: &str = "tpm2_createprimary";
const TPM2_UNSEAL: &str = "tpm2_unseal";
const TPM2_LOAD: &str = "tpm2_load";
const MKTEMP: &str = "mktemp";

// This list of executables required for Clevis to function properly is based
// off of the Clevis dracut module and the Stratis dracut module for supporting
// Clevis in the initramfs. This list is the complete list of executables required
// in the initramfs and so we must check that all of these are present for proper
// Clevis support outside of the initramfs as well.
const CLEVIS_EXEC_NAMES: &[&str] = &[
    CLEVIS,
    CLEVIS_BIND,
    CLEVIS_UNBIND,
    CLEVIS_UNLOCK,
    CLEVIS_LIB,
    CLEVIS_DECRYPT,
    CLEVIS_DECRYPT_TANG,
    CLEVIS_DECRYPT_TPM2,
    CLEVIS_ENCRYPT_TANG,
    CLEVIS_ENCRYPT_TPM2,
    CLEVIS_REGEN,
    JOSE,
    JQ,
    CRYPTSETUP,
    CURL,
    TPM2_CREATEPRIMARY,
    TPM2_UNSEAL,
    TPM2_LOAD,
    MKTEMP,
];

lazy_static! {
    static ref BINARIES: HashMap<String, Option<PathBuf>> = [
        (MKFS_XFS.to_string(), find_binary(MKFS_XFS)),
        (THIN_CHECK.to_string(), find_binary(THIN_CHECK)),
        (THIN_REPAIR.to_string(), find_binary(THIN_REPAIR)),
        (UDEVADM.to_string(), find_binary(UDEVADM)),
        (XFS_DB.to_string(), find_binary(XFS_DB)),
        (XFS_GROWFS.to_string(), find_binary(XFS_GROWFS)),
        (
            THIN_METADATA_SIZE.to_string(),
            find_binary(THIN_METADATA_SIZE)
        ),
    ]
    .iter()
    .cloned()
    .collect();
    static ref CLEVIS_BINARIES: Option<(PathBuf, PathBuf)> = CLEVIS_EXEC_NAMES
        .iter()
        .fold(Some(HashMap::new()), |hm, name| {
            match (hm, find_binary(name)) {
                (None, _) => None,
                (Some(mut hm), Some(path)) => {
                    hm.insert((*name).to_string(), path);
                    Some(hm)
                }
                (_, None) => {
                    info!(
                        "Clevis executable {} not found; disabling Clevis support",
                        name
                    );
                    None
                }
            }
        })
        .and_then(|mut hm| hm
            .remove(CLEVIS)
            .and_then(|c| hm.remove(JOSE).map(|j| (c, j))));
}

/// Verify that all binaries that the engine might invoke are available at some
/// path. Return an error if any are missing. Required to be called on engine
/// initialization.
pub fn verify_binaries() -> StratisResult<()> {
    match BINARIES.iter().find(|&(_, path)| path.is_none()) {
        None => Ok(()),
        Some((name, _)) => Err(StratisError::Msg(format!(
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
        Err(err) => Err(StratisError::Msg(format!(
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
                Err(StratisError::Msg(err_msg))
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

/// Get an absolute path for the Clevis executable or return an error if Clevis
/// support is disabled.
fn get_clevis_executable() -> StratisResult<&'static Path> {
    Ok(CLEVIS_BINARIES.as_ref().map(|(c, _)| c).ok_or_else(|| {
        StratisError::Msg(format!(
            "Clevis has been disabled due to some of the required executables not \
                being found on this system. Required executables are: {:?}",
            CLEVIS_EXEC_NAMES,
        ))
    })?)
}

/// Get an absolute path for the jose executable or return an error if Clevis
/// support is disabled.
fn get_jose_executable() -> StratisResult<&'static Path> {
    Ok(CLEVIS_BINARIES.as_ref().map(|(_, j)| j).ok_or_else(|| {
        StratisError::Msg(format!(
            "Clevis has been disabled due to some of the required executables not \
                being found on this system. Required executables are: {:?}",
            CLEVIS_EXEC_NAMES,
        ))
    })?)
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
    slot: c_uint,
    pin: &str,
    json: &Value,
    yes: bool,
) -> StratisResult<()> {
    let mut cmd = Command::new(get_clevis_executable()?);

    cmd.arg("luks").arg("bind");

    if yes {
        cmd.arg("-y");
    };

    cmd.arg("-d")
        .arg(dev_path.display().to_string())
        .arg("-k")
        .arg(keyfile_path)
        .arg("-t")
        .arg(slot.to_string())
        .arg(pin)
        .arg(json.to_string());

    execute_cmd(&mut cmd)
}

/// Unbind a LUKS device using clevis.
pub fn clevis_luks_unbind(dev_path: &Path, keyslot: libc::c_uint) -> StratisResult<()> {
    execute_cmd(
        Command::new(get_clevis_executable()?)
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
        Command::new(get_clevis_executable()?)
            .arg("luks")
            .arg("unlock")
            .arg("-d")
            .arg(dev_path.display().to_string())
            .arg("-n")
            .arg(dm_name),
    )
}

/// Safely query clevis for the decrypted passphrase stored on a LUKS2 volume.
pub fn clevis_decrypt(jwe: &Value) -> StratisResult<SizedKeyMemory> {
    let mut jose_child = Command::new(get_jose_executable()?)
        .arg("jwe")
        .arg("fmt")
        .arg("-i-")
        .arg("-c")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut jose_stdin = jose_child.stdin.take().ok_or_else(|| {
        StratisError::Msg(
            "Could not communicate with executable {} through stdin; Stratis will \
            not be able to decrypt the Clevis passphrase"
                .to_string(),
        )
    })?;
    jose_stdin.write_all(jwe.to_string().as_bytes())?;

    jose_child.wait()?;

    let mut jose_output = String::new();
    jose_child
        .stdout
        .ok_or_else(|| {
            StratisError::Msg(
                "Spawned jose process had no stdout; cannot continue with password decryption"
                    .to_string(),
            )
        })?
        .read_to_string(&mut jose_output)?;

    let mut clevis_child = Command::new(get_clevis_executable()?)
        .arg("decrypt")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut clevis_stdin = clevis_child.stdin.take().ok_or_else(|| {
        StratisError::Msg(
            "Could not communicate with executable clevis through stdin; Stratis will \
            not be able to decrypt the Clevis passphrase"
                .to_string(),
        )
    })?;
    clevis_stdin.write_all(jose_output.as_bytes())?;
    drop(clevis_stdin);

    clevis_child.wait()?;

    let mut mem = SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE)?;
    let bytes_read = clevis_child
        .stdout
        .ok_or_else(|| {
            StratisError::Msg(
                "Spawned clevis process had no stdout; cannot continue with password decryption"
                    .to_string(),
            )
        })?
        .read(mem.as_mut())?;
    Ok(SizedKeyMemory::new(mem, bytes_read))
}

/// Regenerate the bindings for a device using the clevis CLI.
pub fn clevis_luks_regen(dev_path: &Path, keyslot: c_uint) -> StratisResult<()> {
    execute_cmd(
        Command::new(get_clevis_executable()?)
            .arg("luks")
            .arg("regen")
            .arg("-d")
            .arg(dev_path.display().to_string())
            .arg("-s")
            .arg(keyslot.to_string())
            .arg("-q"),
    )
}

/// Determine the number of sectors required to house the specified parameters for
/// the thin pool that determine metadata size.
pub fn thin_metadata_size(
    block_size: Sectors,
    pool_size: Sectors,
    max_thins: u64,
) -> StratisResult<Sectors> {
    let mut thin_meta_child = Command::new(get_executable(THIN_METADATA_SIZE))
        .arg("-b")
        .arg(format!("{}b", *block_size.bytes()))
        .arg("-s")
        .arg(format!("{}b", *pool_size.bytes()))
        .arg("-m")
        .arg(max_thins.to_string())
        .arg("-n")
        .stdout(Stdio::piped())
        .spawn()?;
    thin_meta_child.wait()?;
    let mut output = String::new();
    let is_ok = thin_meta_child.wait()?.code() == Some(0);
    thin_meta_child
        .stdout
        .ok_or_else(|| {
            StratisError::Msg(
                "Spawned thin_metadata_size process had no stdout; cannot continue with metadata size requirement simulation"
                    .to_string(),
            )
        })?
        .read_to_string(&mut output)?;
    if is_ok {
        Ok(min(
            Sectors(output.trim().parse::<u64>()?),
            MAX_META_SIZE.sectors(),
        ))
    } else {
        Err(StratisError::Msg(format!(
            "thin_metadata_size failed: {}",
            output
        )))
    }
}

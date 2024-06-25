// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Handles invoking external executables.
// This module assumes that, for a given machine, there is only one place
// where the desired executable might be installed. It expects the engine
// to identify that place at its initialization by invoking verify_executables(),
// and to exit immediately if verify_executables() return an error. If this
// protocol is followed then when any command is executed the unique absolute
// path of the executable for this machine will already have been identified.
// However stratisd may run for a while and it is possible for the executable
// to be caused to be uninstalled while stratisd is being run. Therefore,
// the existence of the file is checked before the command is invoked, and
// an explicit error is returned if the executable can not be found.

use std::{
    cmp::min,
    collections::HashMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use either::Either;
use libc::c_uint;
use libcryptsetup_rs::SafeMemHandle;
use once_cell::sync::Lazy;
use semver::{Version, VersionReq};
use serde_json::Value;

use devicemapper::{MetaBlocks, Sectors};

use crate::{
    engine::{
        engine::MAX_STRATIS_PASS_SIZE,
        strat_engine::keys::get_persistent_keyring,
        types::{FilesystemUuid, SizedKeyMemory, StratisUuid},
    },
    stratis::{StratisError, StratisResult},
};

/// The maximum allowable size of the thinpool metadata device
const MAX_META_SIZE: MetaBlocks = MetaBlocks(255 * ((1 << 14) - 64));
/// This constant is an empirical value. Traditional IO patterns that cause break
/// do not cause the metadata to exceed 8 times the prediction by thin_metadata_size.
/// While this works as an upfront estimate for metadata size, we also listen for
/// the low water mark on the metadata device if needed space exceeds 8 times the
/// prediction.
const THIN_META_MULT_FACTOR: u64 = 8;

/// Find the executable with the given name by looking in likely locations.
/// Return None if no executable was found.
/// Search an explicit list of directories rather than the user's PATH
/// environment variable. stratisd may be running when there is no PATH
/// variable set.
fn find_executable(name: &str) -> Option<PathBuf> {
    EXECUTABLES_PATHS
        .iter()
        .map(|pre| [pre, &name.into()].iter().collect::<PathBuf>())
        .find(|path| path.exists())
}

// These are the external executables that stratisd relies on.
// Any change in this list requires a corresponding change to EXECUTABLES
// and vice-versa.
const MKFS_XFS: &str = "mkfs.xfs";
const THIN_CHECK: &str = "thin_check";
const THIN_REPAIR: &str = "thin_repair";
#[cfg(test)]
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

static EXECUTABLES: Lazy<HashMap<String, Option<PathBuf>>> = Lazy::new(|| {
    [
        (MKFS_XFS.to_string(), find_executable(MKFS_XFS)),
        (THIN_CHECK.to_string(), find_executable(THIN_CHECK)),
        (THIN_REPAIR.to_string(), find_executable(THIN_REPAIR)),
        #[cfg(test)]
        (UDEVADM.to_string(), find_executable(UDEVADM)),
        (XFS_DB.to_string(), find_executable(XFS_DB)),
        (XFS_GROWFS.to_string(), find_executable(XFS_GROWFS)),
        (
            THIN_METADATA_SIZE.to_string(),
            find_executable(THIN_METADATA_SIZE),
        ),
    ]
    .iter()
    .cloned()
    .collect()
});

static EXECUTABLES_PATHS: Lazy<Vec<PathBuf>> =
    Lazy::new(|| match std::option_env!("EXECUTABLES_PATHS") {
        Some(paths) => std::env::split_paths(paths).collect(),
        None => ["/usr/sbin", "/sbin", "/usr/bin", "/bin"]
            .iter()
            .map(|p| p.into())
            .collect(),
    });

/// Verify that all executables that the engine might invoke are available at some
/// path. Return an error if any are missing. Required to be called on engine
/// initialization.
pub fn verify_executables() -> StratisResult<()> {
    match EXECUTABLES.iter().find(|&(_, path)| path.is_none()) {
        None => Ok(()),
        Some((name, _)) => Err(StratisError::Msg(format!(
            "Unable to find executable \"{}\" in any of {}",
            name,
            EXECUTABLES_PATHS
                .iter()
                .map(|p| format!("\"{}\"", p.display()))
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
            "Failed to execute command {cmd:?}, err: {err:?}"
        ))),
        Ok(output) => handle_output(cmd, output),
    }
}

/// Handle the output of an executed command. Return an error if invoking the
/// command fails or if the command itself fails.
fn handle_output(cmd: &mut Command, output: Output) -> StratisResult<()> {
    if output.status.success() {
        Ok(())
    } else {
        let exit_reason = output
            .status
            .code()
            .map_or(String::from("process terminated by signal"), |ec| {
                ec.to_string()
            });
        let std_out_txt = String::from_utf8_lossy(&output.stdout);
        let std_err_txt = String::from_utf8_lossy(&output.stderr);
        let err_msg = format!(
            "Command failed: cmd: {cmd:?}, exit reason: {exit_reason} stdout: {std_out_txt} stderr: {std_err_txt}"
        );
        Err(StratisError::Msg(err_msg))
    }
}

/// Get an absolute path for the executable with the given name.
/// Precondition: verify_executables() has already been invoked.
fn get_executable(name: &str) -> &Path {
    EXECUTABLES
        .get(name)
        .expect("name arguments are all constants defined with EXECUTABLES, lookup can not fail")
        .as_ref()
        .expect("verify_executables() was previously called and returned no error")
}

/// Get an absolute path for a Clevis-related executable or return an error if Clevis
/// support is disabled.
fn get_clevis_executable(name: &str) -> StratisResult<PathBuf> {
    let mut execs_location = CLEVIS_EXEC_NAMES
        .iter()
        .map(|name| (name, find_executable(name)))
        .collect::<HashMap<_, _>>();

    let not_found = execs_location
        .iter()
        .filter_map(|(name, val)| if val.is_none() { Some(name) } else { None })
        .collect::<Vec<_>>();

    if not_found.is_empty() {
        Ok(execs_location
            .remove(&name)
            .ok_or_else(|| {
                StratisError::Msg(format!(
                    "Executable {name} is not in the list of Clevis executables tracked by stratisd"
                ))
            })?
            .expect("not_found.is_empty()"))
    } else {
        Err(StratisError::Msg(format!(
            "Clevis has been disabled due to some of the required executables not being found on this system. Required executables that were not found are: {not_found:?}"
        )))
    }
}

// Return the mkfs.xfs version reported by the "-V" option as a string.
fn get_mkfs_xfs_version() -> StratisResult<String> {
    let mut command = Command::new(get_executable(MKFS_XFS).as_os_str())
        .arg("-V")
        .stdout(Stdio::piped())
        .spawn()?;
    command.wait()?;

    let mut output = String::new();
    let is_ok = command.wait()?.code() == Some(0);
    command
        .stdout
        .ok_or_else(|| {
            StratisError::Msg("Could not read string value from xfs.mkfs output".to_string())
        })?
        .read_to_string(&mut output)?;

    if is_ok {
        output
            .trim()
            .strip_prefix("mkfs.xfs version ")
            .ok_or_else(|| {
                StratisError::Msg("Could not parse version string from mkfs.xfs output".to_string())
            })
            .map(|v| v.to_string())
    } else {
        Err(StratisError::Msg(
            "\"mkfs.xfs -V\" returned an error".to_string(),
        ))
    }
}

/// Create a filesystem on devnode. If uuid specified, set the UUID of the
/// filesystem on creation.
pub fn create_fs(devnode: &Path, uuid: Option<StratisUuid>) -> StratisResult<()> {
    // If the version can not be obtained, which is unlikely, assume that the
    // version of mkfs.xfs is new enough to use the nrext64 option. This will
    // become more and more true with time. If the version is not high enough,
    // this will be obvious when the command-line invocation fails.
    // The nrext64 option became available in xfsprogs 5.19.0-rc0 and the
    // larger extent counter size began to be used by default in xfsprogs
    // 6.5.0. This code sets the option when the xfsprogs version is at least
    // 6.0.0, i.e., slightly after it was introduced and a deal before it
    // became the default.
    let use_nrext64_option = match get_mkfs_xfs_version().and_then(|v| {
        v.parse::<Version>()
            .map_err(|_| {
                StratisError::Msg(format!(
                    "Unable to parse version number from version string {v}"
                ))
            })
            .map(|v| {
                VersionReq::parse(">=6.0.0")
                    .expect("req string is valid")
                    .matches(&v)
            })
    }) {
        Ok(val) => val,
        Err(err) => {
            warn!("Unable to read version of mkfs.xfs: {err}; guessing that mkfs.xfs version is high enough to support nrext64 option");
            true
        }
    };

    let mut command = Command::new(get_executable(MKFS_XFS).as_os_str());
    command.arg("-f");
    command.arg(devnode);

    if let Some(uuid) = uuid {
        command.arg("-m");
        command.arg(format!("uuid={uuid}"));
    }

    // Use smaller size extent counters. Note that the size of the extent
    // counters can be increased using an xfs_repair command, although this can
    // be a costly operation. If a filesystem is created with the larger size
    // extent counters, which is the default for mkfs.xfs >= 6.0.0, then it
    // will be unmountable on kernels which do not support the larger size.
    if use_nrext64_option {
        command.arg("-i");
        command.arg("nrext64=0");
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
            .arg(format!("-c uuid {uuid}"))
            .arg(devnode),
    )
}

/// Call thin_check on a thinpool
pub fn thin_check(devnode: &Path) -> StratisResult<()> {
    execute_cmd(Command::new(get_executable(THIN_CHECK).as_os_str()).arg(devnode))
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
#[cfg(test)]
pub fn udev_settle() -> StratisResult<()> {
    execute_cmd(Command::new(get_executable(UDEVADM).as_os_str()).arg("settle"))
}

/// Bind a LUKS device using clevis.
pub fn clevis_luks_bind(
    dev_path: &Path,
    existing_auth: Either<c_uint, SizedKeyMemory>,
    slot: c_uint,
    pin: &str,
    json: &Value,
    yes: bool,
) -> StratisResult<()> {
    let mut cmd = Command::new(get_clevis_executable(CLEVIS)?);

    cmd.arg("luks").arg("bind");

    if yes {
        cmd.arg("-y");
    };

    cmd.arg("-d").arg(dev_path.display().to_string());

    if let Either::Left(token_slot) = existing_auth {
        cmd.arg("-e").arg(token_slot.to_string());
    }

    cmd.arg("-t")
        .arg(slot.to_string())
        .arg(pin)
        .arg(json.to_string());

    match existing_auth {
        Either::Left(_) => {
            get_persistent_keyring()?;
            execute_cmd(&mut cmd)
        }
        Either::Right(ref key) => {
            cmd.stdin(Stdio::piped());
            let mut child = cmd.spawn()?;
            let stdin = child.stdin.as_mut().ok_or_else(|| {
                StratisError::Msg(
                    "Could not provide passphrase to clevis bind via stdin".to_string(),
                )
            })?;
            stdin.write_all(key.as_ref())?;
            stdin.write_all("\n".as_bytes())?;
            let output = child.wait_with_output()?;
            handle_output(&mut cmd, output)
        }
    }
}

/// Unbind a LUKS device using clevis.
pub fn clevis_luks_unbind(dev_path: &Path, keyslot: libc::c_uint) -> StratisResult<()> {
    execute_cmd(
        Command::new(get_clevis_executable(CLEVIS)?)
            .arg("luks")
            .arg("unbind")
            .arg("-d")
            .arg(dev_path.display().to_string())
            .arg("-s")
            .arg(keyslot.to_string())
            .arg("-f"),
    )
}

/// Safely query clevis for the decrypted passphrase stored on a LUKS2 volume.
pub fn clevis_decrypt(jwe: &Value) -> StratisResult<SizedKeyMemory> {
    let mut jose_child = Command::new(get_clevis_executable(JOSE)?)
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

    let mut clevis_child = Command::new(get_clevis_executable(CLEVIS)?)
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
        Command::new(get_clevis_executable(CLEVIS)?)
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
///
/// Precondition: block_size is a power of 2.
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
        let round = block_size - Sectors(1);
        let determined_size = Sectors(
            *(THIN_META_MULT_FACTOR
                * Sectors(
                    output
                        .trim()
                        .parse::<u64>()
                        .map_err(|e| StratisError::Msg(e.to_string()))?,
                )
                + round)
                & !*round,
        );
        assert!(determined_size % block_size == Sectors(0));
        let max = Sectors(*MAX_META_SIZE.sectors() & !*round);
        assert!(max % block_size == Sectors(0));
        Ok(min(determined_size, max))
    } else {
        Err(StratisError::Msg(format!(
            "thin_metadata_size failed: {output}"
        )))
    }
}

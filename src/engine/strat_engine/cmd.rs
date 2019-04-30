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
    process::{Command, Output},
};

use backtrace::Backtrace;
use uuid::Uuid;

use crate::stratis::{StratisError, StratisResult};

const BINARIES_PATHS: [&str; 4] = ["/usr/sbin", "/sbin", "/usr/bin", "/bin"];

#[derive(Debug)]
/// Errors resulting from management of external binaries
pub enum ErrorKind {
    /// Binaries that stratisd relies on for operation not available.
    /// names is the names of all binaries not found.
    BinariesNotFound { names: Vec<String> },

    /// The attempt to execute the external binary failed
    /// cmd is a string representation of the command.
    CommandExecutionFailure { cmd: String },

    /// The commmand itself failed
    CommandFailure { cmd: String, output: Output },
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ErrorKind::BinariesNotFound { names } => {
                write!(f, "binaries not found: [{}]", names.join(" ,"))
            }
            ErrorKind::CommandExecutionFailure { cmd } => {
                write!(f, "failed to execute cmd {}", cmd)
            }
            ErrorKind::CommandFailure { cmd, output } => {
                write!(f, "command {} failed: {:?}", cmd, output)
            }
        }
    }
}

#[derive(Debug)]
/// What relation the component error has to its parent
enum Suberror {
    /// The error occurred before the parent error
    Previous(Box<(dyn std::error::Error + Send)>),
    /// The error is further explained or extended by the parent
    Constituent(Box<(dyn std::error::Error + Send)>),
}

#[derive(Debug)]
pub struct Error {
    // The source of the error, which may be an error for
    // which this error is a further explanation, i.e., a
    // constituent error, or it may simply be an error that occurred
    // previously, and which presumably caused the current code to
    // be run and encounter its own, novel error.
    source_impl: Option<Suberror>,

    // The backtrace at the site the error is returned
    backtrace: Backtrace,

    // Distinguish among different errors with an ErrorKind
    pub specifics: ErrorKind,
}

impl Error {
    fn new(kind: ErrorKind) -> Error {
        Error {
            backtrace: Backtrace::new(),
            source_impl: None,
            specifics: kind,
        }
    }

    /// Return the optional backtrace associated with this error.
    // Note that the function name is our_backtrace, so that it does not
    // conflict with a future possible backtrace function in the Error trait.
    pub fn our_backtrace(&self) -> Option<&Backtrace> {
        Some(&self.backtrace)
    }

    /// Set extension as the extension on this error.
    /// Return the head of the chain, now subsequent.
    pub fn set_extension(self, mut extension: Error) -> Error {
        extension.source_impl = Some(Suberror::Constituent(Box::new(self)));
        extension
    }

    /// Set subsequent as the subsequent error for this error.
    /// Return the head of the chain, now subsequent.
    pub fn set_subsequent(self, mut subsequent: Error) -> Error {
        subsequent.source_impl = Some(Suberror::Previous(Box::new(self)));
        subsequent
    }

    /// Set constituent as the constituent of this error.
    pub fn set_constituent(mut self, constituent: Box<dyn std::error::Error + Send>) -> Error {
        self.source_impl = Some(Suberror::Constituent(constituent));
        self
    }

    /// Set previous as the previous error.
    pub fn set_previous(mut self, previous: Box<dyn std::error::Error + Send>) -> Error {
        self.source_impl = Some(Suberror::Previous(previous));
        self
    }

    /// Obtain the immediate previous error, if there is one
    pub fn previous(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self.source_impl.as_ref() {
            Some(Suberror::Previous(c)) => Some(&**c),
            _ => None,
        }
    }

    /// Obtain the immediate constituent error, if there is one
    pub fn constituent(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self.source_impl.as_ref() {
            Some(Suberror::Constituent(c)) => Some(&**c),
            _ => None,
        }
    }
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Error {
        Error::new(kind)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source_impl.as_ref().map(|c| match c {
            Suberror::Previous(c) => &**c as &(dyn std::error::Error + 'static),
            Suberror::Constituent(c) => &**c as &(dyn std::error::Error + 'static),
        })
    }

    // deprecated in 1.33.0
    // identical to source()
    fn cause(&self) -> Option<&dyn std::error::Error> {
        self.source_impl.as_ref().map(|c| match c {
            Suberror::Previous(c) => &**c as &dyn std::error::Error,
            Suberror::Constituent(c) => &**c as &dyn std::error::Error,
        })
    }
}

// Display only the message associated w/ the specifics.
// Consider the rest to be management baggage.
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.specifics)
    }
}

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
pub fn verify_binaries() -> StratisResult<()> {
    match BINARIES.iter().find(|&(_, ref path)| path.is_none()) {
        None => Ok(()),
        Some((ref name, _)) => Err(StratisError::Error(format!(
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

/// Create a filesystem on devnode.
pub fn create_fs(devnode: &Path, uuid: Uuid) -> StratisResult<()> {
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
pub fn xfs_growfs(mount_point: &Path) -> StratisResult<()> {
    execute_cmd(
        Command::new(get_executable(XFS_GROWFS).as_os_str())
            .arg(mount_point)
            .arg("-d"),
    )
}

/// Set a new UUID for filesystem on the devnode.
pub fn set_uuid(devnode: &Path, uuid: Uuid) -> StratisResult<()> {
    execute_cmd(
        Command::new(get_executable(XFS_DB).as_os_str())
            .arg("-x")
            .arg(format!("-c uuid {}", uuid))
            .arg(&devnode),
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

#[cfg(test)]
pub fn create_ext3_fs(devnode: &Path) -> StratisResult<()> {
    execute_cmd(Command::new("wipefs").arg("-a").arg(&devnode))?;
    execute_cmd(Command::new("mkfs.ext3").arg(&devnode))
}

#[cfg(test)]
#[allow(dead_code)]
pub fn xfs_repair(devnode: &Path) -> StratisResult<()> {
    execute_cmd(Command::new("xfs_repair").arg("-n").arg(&devnode))
}

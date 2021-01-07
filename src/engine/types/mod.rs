// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    borrow::Borrow,
    convert::TryFrom,
    fmt, io,
    ops::Deref,
    path::{Path, PathBuf},
    rc::Rc,
};

mod actions;
mod keys;

pub use crate::engine::types::{
    actions::{
        CreateAction, DeleteAction, EngineAction, MappingCreateAction, RenameAction,
        SetCreateAction, SetDeleteAction, SetUnlockAction,
    },
    keys::{EncryptionInfo, KeyDescription, SizedKeyMemory},
};
use crate::stratis::{ErrorEnum, StratisError, StratisResult};

use uuid::Uuid;

pub type DevUuid = Uuid;
pub type FilesystemUuid = Uuid;
pub type PoolUuid = Uuid;

/// Use Clevis or keyring to unlock LUKS volume.
#[derive(Clone, Copy)]
pub enum UnlockMethod {
    Clevis,
    Keyring,
}

impl<'a> TryFrom<&'a str> for UnlockMethod {
    type Error = StratisError;

    fn try_from(s: &str) -> StratisResult<UnlockMethod> {
        match s {
            "keyring" => Ok(UnlockMethod::Keyring),
            "clevis" => Ok(UnlockMethod::Clevis),
            _ => Err(StratisError::Error(format!(
                "{} is an invalid unlock method",
                s
            ))),
        }
    }
}

/// See Design Doc section 10.2.1 for more details.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockDevState {
    Missing = 0,
    Bad = 1,
    Spare = 2,
    NotInUse = 3,
    InUse = 4,
}

/// A struct that may contain a dbus::Path, or may not, and most certainly
/// doesn't if dbus is compiled out. This avoids littering engine code with
/// conditional code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaybeDbusPath(
    #[cfg(feature = "dbus_enabled")] pub Option<dbus::Path<'static>>,
    #[cfg(not(feature = "dbus_enabled"))] pub Option<()>,
);

/// Blockdev tier. Used to distinguish between blockdevs used for
/// data and blockdevs used for a cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockDevTier {
    Data = 0,
    Cache = 1,
}

/// Redundancy classifications which the engine allows for pools.
#[derive(Debug, Eq, PartialEq)]
#[allow(non_camel_case_types)]
/// Redundancy specification for a pool.
pub enum Redundancy {
    NONE = 0,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
#[allow(clippy::rc_buffer)]
pub struct Name(Rc<String>);

impl Name {
    pub fn new(name: String) -> Name {
        Name(Rc::new(name))
    }

    pub fn to_owned(&self) -> String {
        self.0.deref().to_owned()
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Deref for Name {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for Name {
    fn borrow(&self) -> &str {
        &**self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The type of report for which to query.
///
/// NOTE: `EngineState` is no longer an option and is now supported in the Manager D-Bus API.
/// This report may be fetched through `Report::engine_state_report()`.
///
/// * `ErroredPoolDevices` returns the state of devices that caused an error while
/// attempting to reconstruct a pool.
pub enum ReportType {
    ErroredPoolDevices,
}

impl<'a> TryFrom<&'a str> for ReportType {
    type Error = StratisError;

    fn try_from(name: &str) -> StratisResult<ReportType> {
        match name {
            "errored_pool_report" => Ok(ReportType::ErroredPoolDevices),
            _ => Err(StratisError::Engine(
                ErrorEnum::NotFound,
                format!("Report name {} not understood", name),
            )),
        }
    }
}

#[derive(Clone, Debug)]
pub struct BlockDevPath {
    /// Path to the physical device.
    physical_path: PathBuf,
    /// Optional tuple for encrypted devices. The first path is the internal
    /// path of the logical device and the second is the canonicalized path
    /// of the logical device.
    logical_path: Option<(PathBuf, PathBuf)>,
}

impl BlockDevPath {
    /// Path for a device that only has a physical path.
    pub fn physical_device_path(physical: &Path) -> BlockDevPath {
        BlockDevPath {
            physical_path: physical.to_owned(),
            logical_path: None,
        }
    }

    /// Path for a device that has both an associated physical path and a mapped
    /// logical path.
    pub fn mapped_device_path(physical: &Path, logical: &Path) -> Result<BlockDevPath, io::Error> {
        let logical_path = Some((logical.to_owned(), logical.canonicalize()?));
        Ok(BlockDevPath {
            physical_path: physical.to_owned(),
            logical_path,
        })
    }

    /// Path to the physical device storing the data.
    pub fn physical_path(&self) -> &Path {
        &self.physical_path
    }

    /// Path to the physical or logical device where the Stratis metadata should
    /// be written.
    pub fn metadata_path(&self) -> &Path {
        if let Some((ref path, _)) = self.logical_path {
            path
        } else {
            &self.physical_path
        }
    }

    /// This is the path of devices that will be reported to the user.
    ///
    /// In the case of an encrypted device, this will be the canonicalized path of
    /// the path of the device path where metadata should be written. In the
    /// case of an unencrypted device, it returns the user provided path.
    pub fn user_path(&self) -> &Path {
        if let Some((_, ref path)) = self.logical_path {
            path
        } else {
            &self.physical_path
        }
    }
}

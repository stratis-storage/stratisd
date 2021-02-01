// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    borrow::Borrow,
    convert::TryFrom,
    fmt::{self, Debug, Display},
    hash::Hash,
    ops::Deref,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use crate::engine::types::{
    actions::{
        CreateAction, DeleteAction, EngineAction, MappingCreateAction, RenameAction,
        SetCreateAction, SetDeleteAction, SetUnlockAction,
    },
    keys::{EncryptionInfo, KeyDescription, SizedKeyMemory},
};
use crate::stratis::{ErrorEnum, StratisError, StratisResult};

mod actions;
mod keys;

macro_rules! uuid {
    ($vis:vis $ident:ident) => {
        #[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, Deserialize, Serialize)]
        $vis struct $ident(pub uuid::Uuid);

        impl $ident {
            pub fn new_v4() -> Self {
                $ident(uuid::Uuid::new_v4())
            }

            pub fn parse_str(s: &str) -> $crate::stratis::StratisResult<Self> {
                Ok($ident(uuid::Uuid::parse_str(s)?))
            }

            pub fn nil() -> Self {
                $ident(uuid::Uuid::nil())
            }
        }

        impl std::ops::Deref for $ident {
            type Target = uuid::Uuid;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl std::fmt::Display for $ident {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.0.to_simple_ref(), f)
            }
        }

        impl $crate::engine::types::AsUuid for $ident {}
    }
}

pub trait AsUuid:
    Copy
    + Clone
    + Debug
    + Hash
    + Eq
    + PartialEq
    + for<'a> Deserialize<'a>
    + Serialize
    + Deref<Target = Uuid>
    + Display
{
}

uuid!(pub DevUuid);

uuid!(pub FilesystemUuid);

uuid!(pub PoolUuid);

#[derive(Debug)]
pub enum StratisUuid {
    Dev(DevUuid),
    Fs(FilesystemUuid),
    Pool(PoolUuid),
}

impl Deref for StratisUuid {
    type Target = Uuid;

    fn deref(&self) -> &Self::Target {
        match self {
            StratisUuid::Dev(d) => &*d,
            StratisUuid::Fs(f) => &*f,
            StratisUuid::Pool(p) => &*p,
        }
    }
}

impl Display for StratisUuid {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            StratisUuid::Dev(d) => Display::fmt(d, f),
            StratisUuid::Fs(fs) => Display::fmt(fs, f),
            StratisUuid::Pool(p) => Display::fmt(p, f),
        }
    }
}

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
    /// Path to the device represented by this data structure.
    path: PathBuf,
    /// Reference to the path of the child device of this device.
    child_paths: Vec<Arc<BlockDevPath>>,
}

impl BlockDevPath {
    /// Create a new node in the graph representing a device with no children.
    pub fn leaf(path: PathBuf) -> Arc<Self> {
        Arc::new(BlockDevPath {
            path,
            child_paths: vec![],
        })
    }

    /// Create a new node in the graph representing the devices and their children.
    pub fn node_with_children<I>(path: PathBuf, child_paths: I) -> Arc<Self>
    where
        I: IntoIterator<Item = Arc<Self>>,
    {
        Arc::new(BlockDevPath {
            path,
            child_paths: child_paths.into_iter().collect(),
        })
    }

    /// Get the path of the device associated with the current structure.
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    /// Get the child nodes of this node in the tree.
    pub fn children(&self) -> impl Iterator<Item = Arc<Self>> + '_ {
        self.child_paths.iter().cloned()
    }

    /// Paths of the child devices of this node in the graph.
    pub fn child_paths(&self) -> impl Iterator<Item = &Path> + '_ {
        self.child_paths.iter().map(|child| child.path())
    }
}

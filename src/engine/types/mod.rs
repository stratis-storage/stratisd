// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    borrow::Borrow,
    collections::HashMap,
    convert::TryFrom,
    ffi::OsStr,
    fmt,
    ops::Deref,
    path::{Path, PathBuf},
    sync::Arc,
};

use libudev::EventType;

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
pub struct Name(String);

impl Name {
    pub fn new(name: String) -> Name {
        Name(name)
    }

    pub fn to_owned(&self) -> String {
        self.0.clone()
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
        &self.0
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

/// A sendable event with all of the necessary information for the engine
/// to process a received udev event.
pub struct UdevEngineEvent {
    event_type: EventType,
    device: UdevEngineDevice,
}

impl UdevEngineEvent {
    pub fn event_type(&self) -> EventType {
        self.event_type
    }

    pub fn device(&self) -> &UdevEngineDevice {
        &self.device
    }
}

impl<'a> From<&'a libudev::Event<'a>> for UdevEngineEvent {
    fn from(e: &'a libudev::Event<'a>) -> UdevEngineEvent {
        UdevEngineEvent {
            event_type: e.event_type(),
            device: UdevEngineDevice::from(e.device()),
        }
    }
}

/// A sendable device with all of the necessary information for the engine
/// to process a received udev event.
pub struct UdevEngineDevice {
    is_initialized: bool,
    devnode: Option<PathBuf>,
    devnum: Option<libc::dev_t>,
    properties: HashMap<Box<OsStr>, Box<OsStr>>,
}

impl UdevEngineDevice {
    pub fn is_initialized(&self) -> bool {
        self.is_initialized
    }

    pub fn devnode(&self) -> Option<&Path> {
        self.devnode.as_deref()
    }

    pub fn devnum(&self) -> Option<libc::dev_t> {
        self.devnum
    }

    pub fn property_value<T>(&self, property_name: T) -> Option<&OsStr>
    where
        T: AsRef<OsStr>,
    {
        self.properties
            .get(property_name.as_ref())
            .map(|prop| &**prop)
    }
}

impl<'a> From<&'a libudev::Device<'a>> for UdevEngineDevice {
    fn from(d: &'a libudev::Device<'a>) -> UdevEngineDevice {
        UdevEngineDevice {
            is_initialized: d.is_initialized(),
            devnode: d.devnode().map(|p| p.to_owned()),
            devnum: d.devnum(),
            properties: d
                .properties()
                .map(|prop| (Box::from(prop.name()), Box::from(prop.value())))
                .collect(),
        }
    }
}

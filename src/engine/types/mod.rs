// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    borrow::Borrow,
    collections::HashMap,
    convert::TryFrom,
    ffi::OsStr,
    fmt::{self, Debug, Display},
    hash::Hash,
    ops::Deref,
    path::{Path, PathBuf},
};

use libudev::EventType;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use devicemapper::Bytes;

pub use crate::engine::{
    engine::{Engine, StateDiff},
    structures::Lockable,
    types::{
        actions::{
            Clevis, CreateAction, DeleteAction, EngineAction, Key, MappingCreateAction,
            MappingDeleteAction, RegenAction, RenameAction, SetCreateAction, SetDeleteAction,
            SetUnlockAction,
        },
        keys::{EncryptionInfo, KeyDescription, PoolEncryptionInfo, SizedKeyMemory},
    },
};
use crate::stratis::{StratisError, StratisResult};

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
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.0, f)
            }
        }

        impl $crate::engine::types::AsUuid for $ident {}
    }
}

/// Value representing Clevis config information.
pub type ClevisInfo = (String, Value);

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

#[derive(Debug, PartialEq)]
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StratisUuid::Dev(d) => Display::fmt(d, f),
            StratisUuid::Fs(fs) => Display::fmt(fs, f),
            StratisUuid::Pool(p) => Display::fmt(p, f),
        }
    }
}

/// Use Clevis or keyring to unlock LUKS volume.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq)]
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
            _ => Err(StratisError::Msg(format!(
                "{} is an invalid unlock method",
                s
            ))),
        }
    }
}

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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
            _ => Err(StratisError::Msg(format!(
                "Report name {} not understood",
                name
            ))),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct LockedPoolDevice {
    pub devnode: PathBuf,
    pub uuid: DevUuid,
}

#[derive(Debug, PartialEq)]
pub struct LockedPoolInfo {
    pub info: PoolEncryptionInfo,
    pub devices: Vec<LockedPoolDevice>,
}

/// A sendable event with all of the necessary information for the engine
/// to process a received udev event.
pub struct UdevEngineEvent {
    event_type: EventType,
    device: UdevEngineDevice,
}

impl UdevEngineEvent {
    #[cfg(test)]
    pub fn new(event_type: EventType, device: UdevEngineDevice) -> Self {
        UdevEngineEvent { event_type, device }
    }

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
    #[cfg(test)]
    pub fn new(
        is_initialized: bool,
        devnode: Option<PathBuf>,
        devnum: Option<libc::dev_t>,
        properties: HashMap<Box<OsStr>, Box<OsStr>>,
    ) -> Self {
        UdevEngineDevice {
            is_initialized,
            devnode,
            devnum,
            properties,
        }
    }

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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DevicePath(PathBuf);

impl DevicePath {
    pub fn new(path: &Path) -> StratisResult<Self> {
        Ok(DevicePath(path.canonicalize()?))
    }
}

impl Deref for DevicePath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        self.0.as_path()
    }
}

/// Represents what actions this pool can accept.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone)]
pub enum ActionAvailability {
    /// Full set of actions may be taken
    Full = 0,
    /// No requests via an IPC mechanism may be taken
    NoRequests = 1,
    /// No changes may be made to the pool including background changes
    /// like reacting to devicemapper events
    NoPoolChanges = 2,
}

impl Display for ActionAvailability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                ActionAvailability::Full => "fully_operational",
                ActionAvailability::NoRequests => "no_ipc_requests",
                ActionAvailability::NoPoolChanges => "no_pool_changes",
            }
        )
    }
}

/// Indicates that a property that should be consistent across block devices
/// in a pool may be inconsistent.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum MaybeInconsistent<T> {
    Yes,
    No(T),
}

/// Change in attributes of the thin pool that may need to be reported to the
/// IPC layer.
#[derive(Default, Debug)]
pub struct ThinPoolDiff {
    #[allow(clippy::option_option)]
    pub usage: Option<Option<Bytes>>,
    pub allocated_size: Option<Bytes>,
}

impl ThinPoolDiff {
    /// Returns true if the thin pool information has changed.
    pub fn is_changed(&self) -> bool {
        self.usage.is_some() || self.allocated_size.is_some()
    }
}

/// Represents the difference between two dumped states for a filesystem.
#[derive(Default, Debug)]
pub struct StratFilesystemDiff {
    pub size: Option<Bytes>,
    #[allow(clippy::option_option)]
    pub used: Option<Option<Bytes>>,
}

impl StratFilesystemDiff {
    /// Returns true if the filesystem information has changed.
    pub fn is_changed(&self) -> bool {
        self.size.is_some() || self.used.is_some()
    }
}

/// Represents either a name or a UUID.
#[derive(Debug, Clone, PartialEq)]
pub enum LockKey<U> {
    Name(Name),
    Uuid(U),
}

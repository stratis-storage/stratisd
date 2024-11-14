// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fmt::{self, Debug, Display},
    hash::Hash,
    iter::once,
    ops::Deref,
    path::{Path, PathBuf},
};

use libudev::EventType;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::{self, EnumString, FromRepr};
use uuid::Uuid;

pub use crate::engine::{
    engine::StateDiff,
    structures::Lockable,
    types::{
        actions::{
            Clevis, CreateAction, DeleteAction, EngineAction, GrowAction, Key, MappingCreateAction,
            MappingDeleteAction, PropChangeAction, RegenAction, RenameAction, SetCreateAction,
            SetDeleteAction, SetUnlockAction, StartAction, StopAction, ToDisplay,
        },
        diff::{
            Compare, Diff, PoolDiff, StratBlockDevDiff, StratFilesystemDiff, StratPoolDiff,
            ThinPoolDiff,
        },
        keys::{EncryptionInfo, KeyDescription, PoolEncryptionInfo, SizedKeyMemory},
    },
};
use crate::stratis::{StratisError, StratisResult};

mod actions;
mod diff;
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

#[derive(Debug, Eq, PartialEq)]
pub enum StratisUuid {
    Dev(DevUuid),
    Fs(FilesystemUuid),
    Pool(PoolUuid),
}

impl Deref for StratisUuid {
    type Target = Uuid;

    fn deref(&self) -> &Self::Target {
        match self {
            StratisUuid::Dev(d) => d,
            StratisUuid::Fs(f) => f,
            StratisUuid::Pool(p) => p,
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
#[derive(Serialize, Deserialize, Clone, Copy, Eq, PartialEq, Debug, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum UnlockMethod {
    Clevis,
    Keyring,
    Any,
}

/// Blockdev tier. Used to distinguish between blockdevs used for
/// data and blockdevs used for a cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockDevTier {
    Data = 0,
    Cache = 1,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize)]
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
///   attempting to reconstruct a pool.
#[derive(EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum ReportType {
    StoppedPools,
}

#[derive(Debug, Eq, PartialEq)]
pub struct PoolDevice {
    pub devnode: PathBuf,
    pub uuid: DevUuid,
}

#[derive(Debug, Eq, PartialEq)]
pub struct LockedPoolInfo {
    pub info: PoolEncryptionInfo,
    pub devices: Vec<PoolDevice>,
}

#[derive(Default, Debug, Eq, PartialEq)]
pub struct LockedPoolsInfo {
    pub locked: HashMap<PoolUuid, LockedPoolInfo>,
    pub name_to_uuid: HashMap<Name, PoolUuid>,
    pub uuid_to_name: HashMap<PoolUuid, Name>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct StoppedPoolInfo {
    pub info: Option<PoolEncryptionInfo>,
    pub devices: Vec<PoolDevice>,
    pub metadata_version: Option<StratSigblockVersion>,
    pub features: Option<Features>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct Features {
    pub encryption: bool,
}

#[derive(Default, Debug, Eq, PartialEq)]
pub struct StoppedPoolsInfo {
    pub stopped: HashMap<PoolUuid, StoppedPoolInfo>,
    pub partially_constructed: HashMap<PoolUuid, StoppedPoolInfo>,
    pub name_to_uuid: HashMap<Name, PoolUuid>,
    pub uuid_to_name: HashMap<PoolUuid, Name>,
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

impl From<&libudev::Event> for UdevEngineEvent {
    fn from(e: &libudev::Event) -> UdevEngineEvent {
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

    pub fn property_value<T>(&self, property_name: T) -> Option<StratisResult<&str>>
    where
        T: AsRef<OsStr> + Display,
    {
        self.properties
            .get(property_name.as_ref())
            .map(|value| {
                value.to_str()
                    .ok_or_else(|| {
                        StratisError::Msg(
                            format!(
                                "Unable to convert udev property value with key {} to a string, lossy value is {}",
                                property_name,
                                value.to_string_lossy()
                            ),
                        )
                    })
            })
    }
}

impl From<&libudev::Device> for UdevEngineDevice {
    fn from(d: &libudev::Device) -> UdevEngineDevice {
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
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, strum_macros::Display)]
pub enum ActionAvailability {
    /// Full set of actions may be taken
    #[strum(serialize = "fully_operational")]
    Full = 0,
    /// No requests via an IPC mechanism may be taken
    #[strum(serialize = "no_ipc_requests")]
    NoRequests = 1,
    /// No changes may be made to the pool including background changes
    /// like reacting to devicemapper events
    #[strum(serialize = "no_pool_changes")]
    NoPoolChanges = 2,
}

/// Indicates that a property that should be consistent across block devices
/// in a pool may be inconsistent.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum MaybeInconsistent<T> {
    Yes,
    No(T),
}

impl<T> MaybeInconsistent<Option<T>> {
    pub fn as_ref(&self) -> MaybeInconsistent<Option<&T>> {
        match self {
            MaybeInconsistent::Yes => MaybeInconsistent::Yes,
            MaybeInconsistent::No(opt) => MaybeInconsistent::No(opt.as_ref()),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum PoolIdentifier<U> {
    Name(Name),
    Uuid(U),
}

impl<U> Display for PoolIdentifier<U>
where
    U: Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PoolIdentifier::Name(n) => write!(f, "name {n}"),
            PoolIdentifier::Uuid(u) => write!(f, "UUID {u}"),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum UuidOrConflict {
    Uuid(PoolUuid),
    Conflict(HashSet<PoolUuid>),
}

impl UuidOrConflict {
    fn invariant(&self) -> bool {
        if let UuidOrConflict::Conflict(set) = self {
            set.len() > 1
        } else {
            true
        }
    }

    /// Returns Ok(_) if no conflict was found and Err(_) otherwise.
    pub fn to_result(&self) -> StratisResult<PoolUuid> {
        assert!(self.invariant());
        match self {
            UuidOrConflict::Uuid(u) => Ok(*u),
            UuidOrConflict::Conflict(set) => Err(StratisError::Msg(format!(
                "Found conflicting UUIDs for the same pool name: {}; please use UUID for operation",
                set.iter()
                    .map(|u| u.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }

    /// Add a UUID to the set of conflicts.
    pub fn add(&mut self, uuid: PoolUuid) {
        assert!(self.invariant());
        match self {
            UuidOrConflict::Uuid(u) => {
                if *u != uuid {
                    *self = UuidOrConflict::Conflict(
                        once(*u).chain(once(uuid)).collect::<HashSet<_>>(),
                    );
                }
            }
            UuidOrConflict::Conflict(set) => {
                set.insert(uuid);
            }
        }
    }

    /// Remove a UUID from the set of conflicts. If remove() returns true, the
    /// entire entry should be removed from the HashMap containing names and
    /// potential UUID conflicts.
    pub fn remove(&mut self, uuid: &PoolUuid) -> bool {
        assert!(self.invariant());
        match self {
            UuidOrConflict::Uuid(u) => u == uuid,
            UuidOrConflict::Conflict(set) => {
                set.remove(uuid);
                if set.len() == 1 {
                    let last_elem = set.drain().next().expect("Some(_) checked above");
                    *self = UuidOrConflict::Uuid(last_elem);
                }
                false
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, FromRepr)]
#[repr(u8)]
pub enum StratSigblockVersion {
    V1 = 1,
    V2 = 2,
}

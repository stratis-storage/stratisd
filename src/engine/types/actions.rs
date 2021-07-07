// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Contains types which encode the results of actions requested on an engine,
//! pool, filesystem, or blockdev. Each action type is designed to support
//! idempotency. In every case, the action type is used to indicate the
//! effect of the action at the time the action is requested. The action was
//! completed succesfully; this type indicates what changes had to be made.

use std::fmt::{self, Display};

use crate::engine::{
    engine::Filesystem,
    types::{DevUuid, FilesystemUuid, PoolUuid},
};

/// Return value indicating key operation
pub struct Key;

/// Return value indicating clevis operation
pub struct Clevis;

/// A trait for a generic kind of action. Defines the type of the thing to
/// be changed, and also a method to indicate what changed.
pub trait EngineAction {
    type Return;

    /// Returns whether or not the action changed state.
    fn is_changed(&self) -> bool;

    /// Returns the thing or things changed.
    fn changed(self) -> Option<Self::Return>;
}

#[derive(Debug, PartialEq, Eq)]
/// A single create action.
pub enum CreateAction<T> {
    /// The thing already existed.
    Identity,
    /// The thing did not already exist.
    Created(T),
}

impl<T> EngineAction for CreateAction<T> {
    type Return = T;

    fn is_changed(&self) -> bool {
        !matches!(*self, CreateAction::Identity)
    }

    fn changed(self) -> Option<T> {
        match self {
            CreateAction::Created(t) => Some(t),
            _ => None,
        }
    }
}

impl Display for CreateAction<PoolUuid> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CreateAction::Created(uuid) => {
                write!(f, "Pool with UUID {} was created successfully", uuid)
            }
            CreateAction::Identity => {
                write!(
                    f,
                    "The pool requested for creation is already present; no action taken"
                )
            }
        }
    }
}

impl Display for CreateAction<Clevis> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CreateAction::Created(Clevis) => {
                write!(
                    f,
                    "Pool successfully bound to an unlocking mechanism using clevis"
                )
            }
            CreateAction::Identity => {
                write!(
                    f,
                    "The pool requested for clevis binding is already bound; no action taken"
                )
            }
        }
    }
}

impl Display for CreateAction<Key> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CreateAction::Created(Key) => {
                write!(
                    f,
                    "Pool successfully bound to a passphrase in the kernel keyring"
                )
            }
            CreateAction::Identity => {
                write!(
                    f,
                    "The pool requested for keyring binding is already bound; no action taken"
                )
            }
        }
    }
}

impl Display for CreateAction<(FilesystemUuid, &mut dyn Filesystem)> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CreateAction::Created((uuid, fs)) => {
                write!(
                    f,
                    "Snapshot with UUID {} was created successfully from filesystem with path {}",
                    uuid,
                    fs.devnode().display()
                )
            }
            CreateAction::Identity => {
                write!(
                    f,
                    "The snapshot requested for creation is already present; no action taken"
                )
            }
        }
    }
}

/// Idempotent type representing a create action for a mapping from a key to a value
#[derive(Debug, PartialEq, Eq)]
pub enum MappingCreateAction<T> {
    /// The key did not exist and the key and value are newly created.
    Created(T),
    /// The key and the value were not changed.
    Identity,
    /// The key existed and the value was updated.
    ValueChanged(T),
}

impl Display for MappingCreateAction<Key> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            MappingCreateAction::Created(Key) => write!(f, "Set a new key successfully"),
            MappingCreateAction::Identity => {
                write!(f, "The requested key already exists; no action was taken")
            }
            MappingCreateAction::ValueChanged(Key) => write!(
                f,
                "An existing key was updated with a new value successfully"
            ),
        }
    }
}

impl<T> EngineAction for MappingCreateAction<T> {
    type Return = T;

    fn is_changed(&self) -> bool {
        matches!(
            *self,
            MappingCreateAction::Created(_) | MappingCreateAction::ValueChanged(_)
        )
    }

    fn changed(self) -> Option<T> {
        match self {
            MappingCreateAction::Created(t) | MappingCreateAction::ValueChanged(t) => Some(t),
            _ => None,
        }
    }
}

/// Idempotent type representing a delete action for a mapping from a key to a value
#[derive(Debug, PartialEq, Eq)]
pub enum MappingDeleteAction<T> {
    /// The key and the value were deleted.
    Deleted(T),
    /// The key did not exist.
    Identity,
}

impl<T> EngineAction for MappingDeleteAction<T> {
    type Return = T;

    fn is_changed(&self) -> bool {
        matches!(*self, MappingDeleteAction::Deleted(_))
    }

    fn changed(self) -> Option<T> {
        match self {
            MappingDeleteAction::Deleted(t) => Some(t),
            _ => None,
        }
    }
}

impl Display for MappingDeleteAction<Key> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            MappingDeleteAction::Deleted(_) => {
                write!(f, "A key was deleted successfully")
            }
            MappingDeleteAction::Identity => {
                write!(
                    f,
                    "The key requested for deletion is already absent; no action taken"
                )
            }
        }
    }
}

/// A type for the return type of idempotent unlocking actions.
pub struct SetUnlockAction<T> {
    unlocked: Vec<T>,
}

impl<T> SetUnlockAction<T> {
    /// Create a new return type with newly unlocked resources and resources that
    /// are still locked.
    pub fn new(unlocked: Vec<T>) -> SetUnlockAction<T> {
        SetUnlockAction { unlocked }
    }

    /// Create a new return type where no newly unlocked resources are reported.
    pub fn empty() -> SetUnlockAction<T> {
        SetUnlockAction {
            unlocked: Vec::new(),
        }
    }
}

impl<T> EngineAction for SetUnlockAction<T> {
    type Return = Vec<T>;

    fn is_changed(&self) -> bool {
        !self.unlocked.is_empty()
    }

    fn changed(self) -> Option<Vec<T>> {
        if self.unlocked.is_empty() {
            None
        } else {
            Some(self.unlocked)
        }
    }
}

impl Display for SetUnlockAction<DevUuid> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.unlocked.is_empty() {
            write!(
                f,
                "No new devices were able to be unlocked; no action was taken"
            )
        } else {
            write!(
                f,
                "The devices with UUIDs {} were successfully unlocked",
                self.unlocked
                    .iter()
                    .map(|uuid| uuid.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
/// An action which may create multiple things.
pub struct SetCreateAction<T> {
    changed: Vec<T>,
}

impl<T> SetCreateAction<T> {
    pub fn new(changed: Vec<T>) -> Self {
        SetCreateAction { changed }
    }

    pub fn empty() -> Self {
        SetCreateAction { changed: vec![] }
    }
}

impl<T> EngineAction for SetCreateAction<T> {
    type Return = Vec<T>;

    fn is_changed(&self) -> bool {
        !self.changed.is_empty()
    }

    fn changed(self) -> Option<Vec<T>> {
        if self.changed.is_empty() {
            None
        } else {
            Some(self.changed)
        }
    }
}

impl Display for SetCreateAction<(&str, FilesystemUuid)> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.changed.is_empty() {
            write!(
                f,
                "The requested filesystems already exist; no action taken"
            )
        } else {
            write!(
                f,
                "The following filesystems {} were successfully created",
                self.changed
                    .iter()
                    .map(|(n, u)| format!("name: {}, UUID: {}", n, u))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        }
    }
}

impl Display for SetCreateAction<DevUuid> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.changed.is_empty() {
            write!(
                f,
                "The requested filesystems already exist; no action taken"
            )
        } else {
            write!(
                f,
                "The following devices with UUIDs {} were successfully added to a pool",
                self.changed
                    .iter()
                    .map(|u| u.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
/// An action which may rename a single thing.
pub enum RenameAction<T> {
    /// The thing already had the given name.
    Identity,
    /// The thing did not have the given name and was renamed.
    Renamed(T),
    /// The thing did not exist, so could not be renamed.
    NoSource,
}

impl<T> EngineAction for RenameAction<T> {
    type Return = T;

    fn is_changed(&self) -> bool {
        matches!(*self, RenameAction::Renamed(_))
    }

    fn changed(self) -> Option<T> {
        match self {
            RenameAction::Renamed(t) => Some(t),
            _ => None,
        }
    }
}

impl Display for RenameAction<DevUuid> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RenameAction::Identity => {
                write!(
                    f,
                    "Device is already named the target name; no action taken"
                )
            }
            RenameAction::Renamed(uuid) => {
                write!(f, "Device with UUID {} was successfully renamed", uuid)
            }
            RenameAction::NoSource => {
                write!(f, "The device requested to be renamed does not exist")
            }
        }
    }
}

impl Display for RenameAction<FilesystemUuid> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RenameAction::Identity => {
                write!(
                    f,
                    "Filesystem is already named the target name; no action taken"
                )
            }
            RenameAction::Renamed(uuid) => {
                write!(f, "Filesystem with UUID {} was successfully renamed", uuid)
            }
            RenameAction::NoSource => {
                write!(f, "The filesystem requested to be renamed does not exist")
            }
        }
    }
}

impl Display for RenameAction<PoolUuid> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RenameAction::Identity => {
                write!(f, "Pool is already named the target name; no action taken")
            }
            RenameAction::Renamed(uuid) => {
                write!(f, "Pool with UUID {} was successfully renamed", uuid)
            }
            RenameAction::NoSource => {
                write!(f, "The pool requested to be renamed does not exist")
            }
        }
    }
}

impl Display for RenameAction<Key> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RenameAction::Identity => {
                write!(
                    f,
                    "Requested passphrase in change passphrase operation was the same as the original"
                )
            }
            RenameAction::Renamed(_) => {
                write!(f, "Passphrase was successfully changed")
            }
            RenameAction::NoSource => {
                write!(
                    f,
                    "Could not change the passphrase as no passphrase is currently set"
                )
            }
        }
    }
}

impl Display for RenameAction<Clevis> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RenameAction::Identity => {
                write!(
                    f,
                    "Clevis bindings regeneration resulted in no changes to the metadata",
                )
            }
            RenameAction::Renamed(_) => {
                write!(f, "Clevis bindings were successfully regenerated")
            }
            RenameAction::NoSource => {
                write!(
                    f,
                    "Could not change the Clevis bindings as this pool is not bound to Clevis"
                )
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
/// A single delete action.
pub enum DeleteAction<T> {
    /// The thing was already gone.
    Identity,
    /// The thing existed and was removed.
    Deleted(T),
}

impl<T> EngineAction for DeleteAction<T> {
    type Return = T;

    fn is_changed(&self) -> bool {
        matches!(*self, DeleteAction::Deleted(_))
    }

    fn changed(self) -> Option<T> {
        match self {
            DeleteAction::Deleted(t) => Some(t),
            _ => None,
        }
    }
}

impl Display for DeleteAction<PoolUuid> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DeleteAction::Deleted(uuid) => {
                write!(f, "Pool with UUID {} was deleted successfully", uuid)
            }
            DeleteAction::Identity => {
                write!(
                    f,
                    "The pool requested for deletion is already absent; no action taken"
                )
            }
        }
    }
}

impl Display for DeleteAction<Clevis> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DeleteAction::Deleted(_) => {
                write!(f, "A clevis binding was successfully removed from a pool")
            }
            DeleteAction::Identity => {
                write!(
                    f,
                    "The clevis binding requested for removal is already absent; no action taken"
                )
            }
        }
    }
}

impl Display for DeleteAction<Key> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DeleteAction::Deleted(_) => {
                write!(
                    f,
                    "Bindings to a passphrase in the kernel keyring were removed successfully"
                )
            }
            DeleteAction::Identity => {
                write!(
                    f,
                    "The keyring bindings requested for removal are already absent; no action taken"
                )
            }
        }
    }
}

/// An action which may create multiple things.
#[derive(Debug, PartialEq, Eq)]
pub struct SetDeleteAction<T> {
    changed: Vec<T>,
}

impl<T> SetDeleteAction<T> {
    pub fn new(changed: Vec<T>) -> Self {
        SetDeleteAction { changed }
    }

    pub fn empty() -> Self {
        SetDeleteAction { changed: vec![] }
    }
}

impl<T> EngineAction for SetDeleteAction<T> {
    type Return = Vec<T>;

    fn is_changed(&self) -> bool {
        !self.changed.is_empty()
    }

    fn changed(self) -> Option<Vec<T>> {
        if self.changed.is_empty() {
            None
        } else {
            Some(self.changed)
        }
    }
}

impl Display for SetDeleteAction<FilesystemUuid> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.changed.is_empty() {
            write!(
                f,
                "The requested filesystems are already absent; no action taken"
            )
        } else {
            write!(
                f,
                "Filesystems with UUIDs {} were successfully deleted",
                self.changed
                    .iter()
                    .map(|u| u.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

/// Action indicating a Clevis binding regeneration
pub struct RegenAction;

impl Display for RegenAction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "The Clevis bindings were successfully regenerated using the same configuration that was originally supplied"
        )
    }
}

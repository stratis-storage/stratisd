// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Contains types which encode the results of actions requested on an engine,
//! pool, filesystem, or blockdev. Each action type is designed to support
//! idempotency. In every case, the action type is used to indicate the
//! effect of the action at the time the action is requested. The action was
//! completed succesfully; this type indicates what changes had to be made.

use std::fmt;

use crate::engine::types::PoolUuid;

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
        match *self {
            CreateAction::Identity => false,
            _ => true,
        }
    }

    fn changed(self) -> Option<T> {
        match self {
            CreateAction::Created(t) => Some(t),
            _ => None,
        }
    }
}

impl fmt::Display for CreateAction<PoolUuid> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CreateAction::Created(uuid) => write!(f, "pool UUID {} created", uuid.to_simple_ref()),
            CreateAction::Identity => write!(f, "pool to be created already exists"),
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
        match *self {
            RenameAction::Renamed(_) => true,
            _ => false,
        }
    }

    fn changed(self) -> Option<T> {
        match self {
            RenameAction::Renamed(t) => Some(t),
            _ => None,
        }
    }
}

impl fmt::Display for RenameAction<PoolUuid> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RenameAction::Renamed(uuid) => write!(f, "pool UUID {} renamed", uuid.to_simple_ref()),
            RenameAction::Identity => write!(
                f,
                "rename not performed -- old and new pool names are identical"
            ),
            RenameAction::NoSource => write!(f, "no pool found with old name"),
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
        match *self {
            DeleteAction::Deleted(_) => true,
            _ => false,
        }
    }

    fn changed(self) -> Option<T> {
        match self {
            DeleteAction::Deleted(t) => Some(t),
            _ => None,
        }
    }
}

impl fmt::Display for DeleteAction<PoolUuid> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DeleteAction::Deleted(uuid) => {
                write!(f, "pool with UUID {} destroyed", uuid.to_simple_ref())
            }
            DeleteAction::Identity => write!(f, "pool to be destroyed does not exist"),
        }
    }
}

/// An action which may delete multiple things.
pub type SetDeleteAction<T> = SetCreateAction<T>;

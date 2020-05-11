// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Contains types which encode the results of actions requested on an engine,
//! pool, filesystem, or blockdev. Each action type is designed to support
//! idempotency. In every case, the action type is used to indicate the
//! effect of the action at the time the action is requested. The action was
//! completed succesfully; this type indicates what changes had to be made.

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

#[derive(Debug, PartialEq, Eq)]
pub enum MappingCreateAction<T> {
    Created(T),
    Identity,
    ValueChanged,
}

impl<T> EngineAction for MappingCreateAction<T> {
    type Return = T;

    fn is_changed(&self) -> bool {
        if let MappingCreateAction::Created(_) = *self {
            true
        } else {
            false
        }
    }

    fn changed(self) -> Option<T> {
        if let MappingCreateAction::Created(t) = self {
            Some(t)
        } else {
            None
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

/// An action which may delete multiple things.
pub type SetDeleteAction<T> = SetCreateAction<T>;

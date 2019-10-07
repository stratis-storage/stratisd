// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub trait EngineAction {
    type Return;

    fn changed(self) -> Option<Self::Return>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum CreateAction<T> {
    Identity,
    Created(T),
}

impl<T> EngineAction for CreateAction<T> {
    type Return = T;

    fn changed(self) -> Option<T> {
        match self {
            CreateAction::Created(t) => Some(t),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
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

    fn changed(self) -> Option<Vec<T>> {
        if self.changed.is_empty() {
            None
        } else {
            Some(self.changed)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RenameAction<T> {
    Identity,
    Renamed(T),
    NoSource,
}

impl<T> EngineAction for RenameAction<T> {
    type Return = T;

    fn changed(self) -> Option<T> {
        match self {
            RenameAction::Renamed(t) => Some(t),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DeleteAction<T> {
    Identity,
    Deleted(T),
}

impl<T> EngineAction for DeleteAction<T> {
    type Return = T;

    fn changed(self) -> Option<T> {
        match self {
            DeleteAction::Deleted(t) => Some(t),
            _ => None,
        }
    }
}

pub type SetDeleteAction<T> = SetCreateAction<T>;

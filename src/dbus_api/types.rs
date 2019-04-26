// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cell::{Cell, RefCell};
use std::collections::vec_deque::{Drain, VecDeque};
use std::rc::Rc;

use dbus::tree::{DataType, MTFn, ObjectPath, Tree};
use dbus::Path;

use uuid::Uuid;

use crate::engine::Engine;

#[derive(Clone, Copy, Debug)]
#[allow(non_camel_case_types)]
pub enum DbusErrorEnum {
    OK = 0,
    ERROR = 1,

    ALREADY_EXISTS = 2,
    BUSY = 3,
    INTERNAL_ERROR = 4,
    NOTFOUND = 5,
}

impl DbusErrorEnum {
    pub fn get_error_string(self) -> &'static str {
        match self {
            DbusErrorEnum::OK => "Ok",
            DbusErrorEnum::ERROR => "A general error happened",
            DbusErrorEnum::ALREADY_EXISTS => "Already exists",
            DbusErrorEnum::BUSY => "Operation can not be performed at this time",
            DbusErrorEnum::INTERNAL_ERROR => "Internal error",
            DbusErrorEnum::NOTFOUND => "Not found",
        }
    }
}

#[derive(Debug)]
pub enum DeferredAction {
    Add(ObjectPath<MTFn<TData>, TData>),
    Remove(Path<'static>),
}

/// Context for an object path.
/// Contains the object path of the parent as a Path and the UUID of the
/// object itself.
#[derive(Debug)]
pub struct OPContext {
    pub(super) parent: Path<'static>,
    pub(super) uuid: Uuid,
}

impl OPContext {
    pub fn new(parent: Path<'static>, uuid: Uuid) -> OPContext {
        OPContext { parent, uuid }
    }
}

#[derive(Debug, Clone)]
pub struct DbusContext {
    pub(super) next_index: Rc<Cell<u64>>,
    pub(super) engine: Rc<RefCell<dyn Engine>>,
    pub(super) actions: Rc<RefCell<ActionQueue>>,
}

impl DbusContext {
    pub fn new(engine: Rc<RefCell<dyn Engine>>) -> DbusContext {
        DbusContext {
            actions: Rc::new(RefCell::new(ActionQueue::default())),
            engine,
            next_index: Rc::new(Cell::new(0)),
        }
    }

    /// Generates a new id for object paths.
    /// It is assumed that, while Stratisd is running, it will never generate
    /// more than 2^64 object paths. If it turns out that this is a bad
    /// assumption, the solution is to use unbounded integers.
    pub fn get_next_id(&self) -> u64 {
        self.next_index.set(self.next_index.get() + 1);
        self.next_index.get()
    }
}

#[derive(Default, Debug)]
pub struct TData;
impl DataType for TData {
    type Tree = DbusContext;
    type ObjectPath = Option<OPContext>;
    type Property = ();
    type Interface = ();
    type Method = ();
    type Signal = ();
}

/// An action queue.
/// Add and remove actions are pushed onto the queue.
/// The queue can also be drained.
#[derive(Debug, Default)]
pub struct ActionQueue {
    queue: VecDeque<DeferredAction>,
}

impl ActionQueue {
    /// Push an Add action onto the back of the queue.
    pub fn push_add(&mut self, object_path: ObjectPath<MTFn<TData>, TData>) {
        self.queue.push_back(DeferredAction::Add(object_path))
    }

    /// Push Remove actions for a path and its immediate descendants. Not
    /// recursive, since no multi-level parent-child relationships currently
    /// exist.
    // Note: Path x is a child of path y if x's context's parent field is y.
    pub fn push_remove(&mut self, item: &Path<'static>, tree: &Tree<MTFn<TData>, TData>) {
        for opath in tree.iter().filter(|opath| {
            opath
                .get_data()
                .as_ref()
                .map_or(false, |op_cxt| op_cxt.parent == *item)
        }) {
            self.queue
                .push_back(DeferredAction::Remove(opath.get_name().clone()))
        }

        self.queue.push_back(DeferredAction::Remove(item.clone()))
    }

    /// Drain the queue.
    pub fn drain(&mut self) -> Drain<DeferredAction> {
        self.queue.drain(..)
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cell::{Cell, RefCell};
use std::collections::vec_deque::{Drain, VecDeque};
use std::convert::From;
use std::rc::Rc;

use dbus::Path;
use dbus::tree::{DataType, MTFn, ObjectPath};

use uuid::Uuid;

use engine::Engine;

custom_derive! {
    #[derive(Copy, Clone, EnumDisplay,
             IterVariants(StratisDBusErrorVariants),
             IterVariantNames(StratisDBusErrorVariantNames))]
    #[allow(non_camel_case_types)]
    pub enum DbusErrorEnum {
        OK,
        ERROR,

        ALREADY_EXISTS,
        BUSY,
        IO_ERROR,
        INTERNAL_ERROR,
        NIX_ERROR,
        NOTFOUND,
    }
}

/// Get the u16 value of this ErrorEnum constructor.
impl From<DbusErrorEnum> for u16 {
    fn from(e: DbusErrorEnum) -> u16 {
        e as u16
    }
}

impl DbusErrorEnum {
    pub fn get_error_string(&self) -> &str {
        match *self {
            DbusErrorEnum::OK => "Ok",
            DbusErrorEnum::ERROR => "A general error happened",
            DbusErrorEnum::ALREADY_EXISTS => "Already exists",
            DbusErrorEnum::BUSY => "Operation can not be performed at this time",
            DbusErrorEnum::INTERNAL_ERROR => "Internal error",
            DbusErrorEnum::IO_ERROR => "IO error during operation",
            DbusErrorEnum::NIX_ERROR => "System error during operation",
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
    pub parent: Path<'static>,
    pub uuid: Uuid,
}

impl OPContext {
    pub fn new(parent: Path<'static>, uuid: Uuid) -> OPContext {
        OPContext {
            parent: parent,
            uuid: uuid,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DbusContext {
    pub next_index: Rc<Cell<u64>>,
    pub engine: Rc<RefCell<Engine>>,
    pub actions: Rc<RefCell<ActionQueue>>,
}

impl DbusContext {
    pub fn new(engine: Rc<RefCell<Engine>>) -> DbusContext {
        DbusContext {
            actions: Rc::new(RefCell::new(ActionQueue::new())),
            engine: engine,
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
    type ObjectPath = Option<OPContext>;
    type Property = ();
    type Interface = ();
    type Method = ();
    type Signal = ();
    type Tree = DbusContext;
}

/// An action queue.
/// Add and remove actions are pushed onto the queue.
/// The queue can also be drained.
#[derive(Debug)]
#[allow(new_without_default_derive)]
pub struct ActionQueue {
    queue: VecDeque<DeferredAction>,
}

impl ActionQueue {
    /// Initialize an empty action queue.
    pub fn new() -> ActionQueue {
        ActionQueue { queue: VecDeque::new() }
    }

    /// Push an Add action onto the back of the queue.
    pub fn push_add(&mut self, object_path: ObjectPath<MTFn<TData>, TData>) {
        self.queue.push_back(DeferredAction::Add(object_path))
    }

    /// Push a Remove action onto the back of the queue.
    pub fn push_remove(&mut self, object_path: Path<'static>) {
        self.queue.push_back(DeferredAction::Remove(object_path))
    }

    /// Drain the queue.
    pub fn drain(&mut self) -> Drain<DeferredAction> {
        self.queue.drain(..)
    }
}

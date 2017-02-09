// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cell::{Cell, RefCell};
use std::collections::vec_deque::{Drain, VecDeque};
use std::convert::From;
use std::rc::Rc;

use bidir_map::BidirMap;

use dbus::Path;
use dbus::tree::{DataType, MTFn, ObjectPath};

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
        BAD_PARAM,
        BUSY,
        CACHE_NOTFOUND,
        DEV_NOTFOUND,
        FILESYSTEM_NOTFOUND,
        IO_ERROR,
        LIST_FAILURE,
        INTERNAL_ERROR,
        NIX_ERROR,
        NO_POOLS,
        NOTFOUND,
        NULL,
        NULL_NAME,
        POOL_NOTFOUND,
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
            DbusErrorEnum::NULL => "Null parameter was supplied",
            DbusErrorEnum::NOTFOUND => "Not found",
            DbusErrorEnum::POOL_NOTFOUND => "Pool not found",
            DbusErrorEnum::FILESYSTEM_NOTFOUND => "Filesystem not found",
            DbusErrorEnum::CACHE_NOTFOUND => "Cache not found",
            DbusErrorEnum::BAD_PARAM => "Bad parameter",
            DbusErrorEnum::DEV_NOTFOUND => "Dev not found",
            DbusErrorEnum::ALREADY_EXISTS => "Already exists",
            DbusErrorEnum::NULL_NAME => "Null name supplied",
            DbusErrorEnum::NO_POOLS => "No pools",
            DbusErrorEnum::LIST_FAILURE => "List operation failure",
            DbusErrorEnum::INTERNAL_ERROR => "Internal error",
            DbusErrorEnum::IO_ERROR => "IO error during operation",
            DbusErrorEnum::NIX_ERROR => "System error during operation",
            DbusErrorEnum::BUSY => "Operation can not be performed at this time",
        }
    }
}

#[derive(Debug)]
pub enum DeferredAction {
    Add(ObjectPath<MTFn<TData>, TData>),
    Remove(String),
}

#[derive(Debug, Clone)]
pub struct DbusContext {
    pub next_index: Rc<Cell<u64>>,
    pub pools: Rc<RefCell<PoolMap>>,
    pub engine: Rc<RefCell<Box<Engine>>>,
    pub actions: Rc<RefCell<ActionQueue>>,
    pub filesystems: Rc<RefCell<FilesystemMap>>,
}

impl DbusContext {
    pub fn new(engine: Box<Engine>) -> DbusContext {
        DbusContext {
            actions: Rc::new(RefCell::new(ActionQueue::new())),
            engine: Rc::new(RefCell::new(engine)),
            filesystems: Rc::new(RefCell::new(FilesystemMap::new())),
            next_index: Rc::new(Cell::new(0)),
            pools: Rc::new(RefCell::new(PoolMap::new())),
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
    type ObjectPath = ();
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
    pub fn push_remove(&mut self, object_path: String) {
        self.queue.push_back(DeferredAction::Remove(object_path))
    }

    /// Drain the queue.
    pub fn drain(&mut self) -> Drain<DeferredAction> {
        self.queue.drain(..)
    }
}

/// A map from pool object paths to pool names.
/// An invariant of this map is that all the object paths are convertable to
/// dbus::Path.
/// Since this is just a lookup table, key/value pairs to insert are passed
/// by reference and copied within the insert() function so that the table
/// does not take ownership of the arguments.
/// For the same reason, the get* methods return a thing, rather
/// than a reference to a thing, so that it can be used immediately by the
/// caller.
#[derive(Debug)]
pub struct PoolMap {
    map: BidirMap<String, String>,
}

impl PoolMap {
    pub fn new() -> PoolMap {
        PoolMap { map: BidirMap::new() }
    }

    /// Get name by using the object path.
    pub fn get_by_path(&self, path: &Path) -> Option<String> {
        self.map.get_by_first(path as &str).map(|x| x.clone())
    }

    /// Get object path by using the name.
    pub fn get_by_name(&self, name: &str) -> Option<Path<'static>> {
        self.map.get_by_second(name).and_then(|x| Some(Path::new(x as &str).unwrap()))
    }

    /// Insert a key1/key2 pair.
    /// Returns the supplanted pair, if one exists.
    /// A pool rename might supplant the old entry for path.
    pub fn insert(&mut self, path: &Path, name: &str) -> Option<(Path<'static>, String)> {
        self.map
            .insert((**path).into(), name.into())
            .and_then(|x| Some((Path::new(&x.0 as &str).unwrap(), x.1)))
    }

    /// Remove by name.
    pub fn remove_by_name(&mut self, name: &str) -> Option<(Path<'static>, String)> {
        self.map.remove_by_second(name).and_then(|x| Some((Path::new(&x.0 as &str).unwrap(), x.1)))
    }
}

/// A map from filesystem object paths to pairs of pool name and fs name.
/// The principals are the same as those for the PoolMap.
#[derive(Debug)]
pub struct FilesystemMap {
    map: BidirMap<String, (String, String)>,
}

impl FilesystemMap {
    pub fn new() -> FilesystemMap {
        FilesystemMap { map: BidirMap::new() }
    }

    /// Get name pair by using the object path.
    pub fn get_by_path(&self, path: &Path) -> Option<(String, String)> {
        self.map.get_by_first(path as &str).map(|x| (x.0.clone(), x.1.clone()))
    }

    /// Get object path by using the name pair.
    pub fn get_by_names(&self, pool_name: &str, fs_name: &str) -> Option<Path<'static>> {
        self.map
            .get_by_second(&(pool_name.into(), fs_name.into()))
            .and_then(|x| Some(Path::new(x as &str).unwrap()))
    }

    /// Insert a key1/key2 pair.
    /// Returns the supplanted pair, if one exists.
    /// A pool rename might supplant the old entry for a path.
    pub fn insert(&mut self,
                  path: &Path,
                  pool_name: &str,
                  fs_name: &str)
                  -> Option<(Path<'static>, (String, String))> {
        self.map
            .insert((**path).into(), (pool_name.into(), fs_name.into()))
            .and_then(|x| Some((Path::new(&x.0 as &str).unwrap(), x.1)))
    }

    /// Remove by name pair.
    pub fn remove_by_names(&mut self,
                           pool_name: &str,
                           fs_name: &str)
                           -> Option<(Path<'static>, (String, String))> {
        self.map
            .remove_by_second(&(pool_name.into(), fs_name.into()))
            .and_then(|x| Some((Path::new(&x.0 as &str).unwrap(), x.1)))
    }
}

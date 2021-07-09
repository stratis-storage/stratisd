// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    any::type_name,
    collections::HashMap,
    fmt::{self, Debug},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use dbus::{
    arg::{RefArg, Variant},
    blocking::SyncConnection,
    Path,
};
use dbus_tree::{DataType, MTSync, ObjectPath, Tree};
use tokio::sync::{mpsc::UnboundedSender as TokioSender, RwLock};

use crate::engine::{Lockable, LockableEngine, StratisUuid};

/// Type for lockable D-Bus tree object.
pub type LockableTree = Lockable<Arc<RwLock<Tree<MTSync<TData>, TData>>>>;

/// Type for return value of `GetManagedObjects`.
pub type GetManagedObjects =
    HashMap<dbus::Path<'static>, HashMap<String, HashMap<String, Variant<Box<dyn RefArg>>>>>;

/// Type for interfaces parameter for `ObjectManagerInterfacesAdded`. This type cannot be sent
/// over the D-Bus but it is safe to send across threads.
pub type InterfacesAddedThreadSafe =
    HashMap<String, HashMap<String, Variant<Box<dyn RefArg + Send + Sync>>>>;
/// Type for interfaces parameter for `ObjectManagerInterfacesAdded` that can be written to the
/// D-Bus.
pub type InterfacesAdded = HashMap<String, HashMap<String, Variant<Box<dyn RefArg>>>>;
/// Type for interfaces parameter for `ObjectManagerInterfacesRemoved`.
pub type InterfacesRemoved = Vec<String>;

#[derive(Clone, Copy, Debug)]
#[allow(non_camel_case_types)]
pub enum DbusErrorEnum {
    OK = 0,
    ERROR = 1,
}

pub const OK_STRING: &str = "Ok";

#[derive(Debug)]
pub enum DbusAction {
    Add(ObjectPath<MTSync<TData>, TData>, InterfacesAddedThreadSafe),
    Remove(Path<'static>, InterfacesRemoved),
    FsNameChange(Path<'static>, String),
    PoolNameChange(Path<'static>, String),
}

/// Context for an object path.
/// Contains the object path of the parent as a Path and the UUID of the
/// object itself.
#[derive(Debug)]
pub struct OPContext {
    pub(super) parent: Path<'static>,
    pub(super) uuid: StratisUuid,
}

impl OPContext {
    pub fn new(parent: Path<'static>, uuid: StratisUuid) -> OPContext {
        OPContext { parent, uuid }
    }
}

#[derive(Clone)]
pub struct DbusContext {
    next_index: Arc<AtomicU64>,
    pub(super) engine: LockableEngine,
    pub(super) sender: TokioSender<DbusAction>,
    connection: Arc<SyncConnection>,
}

impl Debug for DbusContext {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("DbusContext")
            .field("next_index", &self.next_index)
            .field("engine", &type_name::<LockableEngine>())
            .field("sender", &self.sender)
            .finish()
    }
}

impl DbusContext {
    pub fn new(
        engine: LockableEngine,
        sender: TokioSender<DbusAction>,
        connection: Arc<SyncConnection>,
    ) -> DbusContext {
        DbusContext {
            engine,
            next_index: Arc::new(AtomicU64::new(0)),
            sender,
            connection,
        }
    }

    /// Generates a new id for object paths.
    /// It is assumed that, while Stratisd is running, it will never generate
    /// more than 2^64 object paths. If it turns out that this is a bad
    /// assumption, the solution is to use unbounded integers.
    pub fn get_next_id(&self) -> u64 {
        self.next_index.fetch_add(1, Ordering::SeqCst)
    }

    pub fn push_add(
        &self,
        object_path: ObjectPath<MTSync<TData>, TData>,
        interfaces: InterfacesAddedThreadSafe,
    ) {
        let object_path_name = object_path.get_name().clone();
        if let Err(e) = self.sender.send(DbusAction::Add(object_path, interfaces)) {
            warn!(
                "D-Bus add event could not be sent to the processing thread; the D-Bus \
                server will not be aware of the D-Bus object with path {}: {}",
                object_path_name, e,
            )
        }
    }

    pub fn push_remove(&self, item: &Path<'static>, interfaces: InterfacesRemoved) {
        if let Err(e) = self
            .sender
            .send(DbusAction::Remove(item.clone(), interfaces))
        {
            warn!(
                "D-Bus remove event could not be sent to the processing thread; the D-Bus \
                server will still expect the D-Bus object with path {} to be present: {}",
                item, e,
            )
        }
    }

    /// Send changed signal for Name property and invalidated signal for
    /// Devnode property.
    pub fn push_filesystem_name_change(&self, item: &Path<'static>, new_name: &str) {
        if let Err(e) = self
            .sender
            .send(DbusAction::FsNameChange(item.clone(), new_name.to_string()))
        {
            warn!(
                "D-Bus filesystem name change event could not be sent to the processing thread; \
                no signal will be sent out for pool with path {}: {}",
                item, e,
            )
        }
    }

    /// Send changed signal for pool Name property and invalidated signal for
    /// all Devnode properties of child filesystems.
    pub fn push_pool_name_change(&self, item: &Path<'static>, new_name: &str) {
        if let Err(e) = self
            .sender
            .send(DbusAction::FsNameChange(item.clone(), new_name.to_string()))
        {
            warn!(
                "D-Bus pool name change event could not be sent to the processing thread; \
                no signal will be sent out for the name change of pool with path {} or any \
                of its child filesystems: {}",
                item, e,
            )
        }
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

/// Tri-state enum to determine CreatePool parameters accepted by the D-Bus method.
pub enum CreatePoolParams {
    #[allow(dead_code)]
    Neither,
    #[allow(dead_code)]
    KeyDesc,
    Both,
}

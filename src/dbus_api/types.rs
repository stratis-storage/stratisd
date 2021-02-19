// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::HashMap,
    fmt::{self, Debug},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use dbus::{
    arg::{RefArg, Variant},
    tree::{DataType, MTSync, ObjectPath, Tree},
    Path,
};
use futures::executor::block_on;
use tokio::sync::{mpsc::Sender, Mutex};

use crate::{
    dbus_api::consts,
    engine::{Engine, StratisUuid},
};

/// Type for interfaces parameter for `ObjectManagerInterfacesAdded`.
pub type InterfacesAdded = HashMap<String, HashMap<String, Variant<Box<dyn RefArg + Send + Sync>>>>;
/// Type for interfaces parameter for `ObjectManagerInterfacesRemoved`.
pub type InterfacesRemoved = Vec<String>;

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
pub enum DbusAction {
    Add(ObjectPath<MTSync<TData>, TData>, InterfacesAdded),
    InvalidateFilesystemDevnode(Path<'static>),
    ChangeFilesystemName(Path<'static>, String),
    ChangePoolName(Path<'static>, String),
    Remove(Path<'static>, InterfacesRemoved),
}

/// Indicates the type of object pointed to by the object path.
#[derive(Debug)]
pub enum ObjectPathType {
    Pool,
    Filesystem,
    Blockdev,
}

/// Context for an object path.
/// Contains the object path of the parent as a Path and the UUID of the
/// object itself.
#[derive(Debug)]
pub struct OPContext {
    pub(super) parent: Path<'static>,
    pub(super) uuid: StratisUuid,
    pub(super) op_type: ObjectPathType,
}

impl OPContext {
    pub fn new(parent: Path<'static>, uuid: StratisUuid, op_type: ObjectPathType) -> OPContext {
        OPContext {
            parent,
            uuid,
            op_type,
        }
    }
}

#[derive(Clone)]
pub struct DbusContext {
    next_index: Arc<AtomicU64>,
    pub(super) engine: Arc<Mutex<dyn Engine>>,
    pub(super) sender: Sender<DbusAction>,
}

impl Debug for DbusContext {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "DbusContext {{ next_index: {:?}, engine: Arc<Mutex<dyn Engine>>, \
            sender: {:?} }}",
            self.next_index, self.sender,
        )
    }
}

impl DbusContext {
    pub fn new(engine: Arc<Mutex<dyn Engine>>, sender: Sender<DbusAction>) -> DbusContext {
        DbusContext {
            engine,
            next_index: Arc::new(AtomicU64::new(0)),
            sender,
        }
    }

    /// Generates a new id for object paths.
    /// It is assumed that, while Stratisd is running, it will never generate
    /// more than 2^64 object paths. If it turns out that this is a bad
    /// assumption, the solution is to use unbounded integers.
    pub fn get_next_id(&self) -> u64 {
        let index = self.next_index.load(Ordering::Acquire) + 1;
        self.next_index.store(index, Ordering::Release);
        index
    }

    pub fn push_add(
        &self,
        object_path: ObjectPath<MTSync<TData>, TData>,
        interfaces: InterfacesAdded,
    ) {
        let object_path_name = object_path.get_name().clone();
        if let Err(e) = block_on(self.sender.send(DbusAction::Add(object_path, interfaces))) {
            warn!(
                "D-Bus add event could not be sent to the processing thread; the D-Bus \
                server will not be aware of the D-Bus object with path {}: {}",
                object_path_name, e,
            )
        }
    }

    pub fn push_filesystem_name_change(&self, object_path: &Path<'static>, new_name: &str) {
        if let Err(e) = block_on(self.sender.send(DbusAction::ChangeFilesystemName(
            object_path.clone(),
            new_name.to_owned(),
        ))) {
            warn!(
                "D-Bus filesystem name change event could not be sent to the processing thread; the D-Bus \
                server will not be aware of the new name of the D-Bus object with path {}: {}",
                object_path, e,
            )
        }
        if let Err(e) = block_on(
            self.sender
                .send(DbusAction::InvalidateFilesystemDevnode(object_path.clone())),
        ) {
            warn!(
                "D-Bus filesystem devnode invalidated event could not be sent to the processing thread; the D-Bus \
                server will not be aware of the new devnode of the D-Bus object with path {}: {}",
                object_path, e,
            )
        }
    }

    pub fn push_pool_name_change(
        &self,
        object_path: &Path<'static>,
        new_name: &str,
        tree: &Tree<MTSync<TData>, TData>,
    ) {
        if let Err(e) = block_on(self.sender.send(DbusAction::ChangePoolName(
            object_path.clone(),
            new_name.to_owned(),
        ))) {
            warn!(
                "D-Bus pool name change event could not be sent to the processing thread; the D-Bus \
                server will not be aware of the new name of the D-Bus object with path {}: {}",
                object_path, e,
            )
        }

        for opath in tree.iter().filter(|opath| {
            opath
                .get_data()
                .as_ref()
                .map_or(false, |op_cxt| op_cxt.parent == *object_path)
        }) {
            if let ObjectPathType::Filesystem = opath
                .get_data()
                .as_ref()
                .expect("all objects with parents have data")
                .op_type
            {
                if let Err(e) = block_on(self.sender.send(DbusAction::InvalidateFilesystemDevnode(
                    opath.get_name().clone(),
                ))) {
                    warn!(
                        "D-Bus filesystem devnode invalidated event could not be sent to the processing thread; the D-Bus \
                        server will not be aware of the new devnode of the D-Bus object with path {}: {}",
                        opath.get_name(), e,
                    )
                }
            }
        }
    }

    pub fn push_remove(
        &self,
        item: &Path<'static>,
        tree: &Tree<MTSync<TData>, TData>,
        interfaces: InterfacesRemoved,
    ) {
        for opath in tree.iter().filter(|opath| {
            opath
                .get_data()
                .as_ref()
                .map_or(false, |op_cxt| op_cxt.parent == *item)
        }) {
            if let Err(e) = block_on(
                self.sender.send(DbusAction::Remove(
                    opath.get_name().clone(),
                    match opath
                        .get_data()
                        .as_ref()
                        .expect("all objects with parents have data")
                        .op_type
                    {
                        ObjectPathType::Pool => consts::pool_interface_list(),
                        ObjectPathType::Filesystem => consts::filesystem_interface_list(),
                        ObjectPathType::Blockdev => consts::blockdev_interface_list(),
                    },
                )),
            ) {
                warn!(
                    "D-Bus remove event could not be sent to the processing thread; the D-Bus \
                    server will still expect the D-Bus object with path {} to be present: {}",
                    opath.get_name(),
                    e,
                )
            }
        }

        if let Err(e) = block_on(
            self.sender
                .send(DbusAction::Remove(item.clone(), interfaces)),
        ) {
            warn!(
                "D-Bus remove event could not be sent to the processing thread; the D-Bus \
                server will still expect the D-Bus object with path {} to be present: {}",
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

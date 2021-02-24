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
    blocking::{
        stdintf::org_freedesktop_dbus::{
            ObjectManagerInterfacesAdded, ObjectManagerInterfacesRemoved,
        },
        SyncConnection,
    },
    channel::Sender,
    message::SignalArgs,
    Path,
};
use dbus_tree::{DataType, MTSync, ObjectPath, Tree};
use futures::executor::block_on;
use tokio::sync::{mpsc::Sender as TokioSender, Mutex};

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
    Add(ObjectPath<MTSync<TData>, TData>),
    Remove(Vec<Path<'static>>),
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
    pub(super) next_index: Arc<AtomicU64>,
    pub(super) engine: Arc<Mutex<dyn Engine>>,
    pub(super) sender: TokioSender<DbusAction>,
    connection: Arc<SyncConnection>,
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
    pub fn new(
        engine: Arc<Mutex<dyn Engine>>,
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

    pub fn push_add(
        &self,
        object_path: ObjectPath<MTSync<TData>, TData>,
        interfaces: InterfacesAdded,
    ) {
        let object_path_name = object_path.get_name().clone();
        if self
            .added_object_signal(object_path_name.clone(), interfaces)
            .is_err()
        {
            warn!("Signal on object add was not sent to the D-Bus client");
        }
        if let Err(e) = block_on(self.sender.send(DbusAction::Add(object_path))) {
            warn!(
                "D-Bus add event could not be sent to the processing thread; the D-Bus \
                server will not be aware of the D-Bus object with path {}: {}",
                object_path_name, e,
            )
        }
    }

    /// Send an InterfacesAdded signal on the D-Bus
    fn added_object_signal(
        &self,
        object: Path<'static>,
        interfaces: InterfacesAdded,
    ) -> Result<(), dbus::Error> {
        self.connection
            .send(
                ObjectManagerInterfacesAdded {
                    object,
                    interfaces: interfaces
                        .into_iter()
                        .map(|(k, map)| {
                            let new_map: HashMap<String, Variant<Box<dyn RefArg>>> = map
                                .into_iter()
                                .map(|(subk, var)| (subk, Variant(var.0 as Box<dyn RefArg>)))
                                .collect();
                            (k, new_map)
                        })
                        .collect(),
                }
                .to_emit_message(&Path::from(consts::STRATIS_BASE_PATH)),
            )
            .map(|_| ())
            .map_err(|_| {
                dbus::Error::new_failed("Failed to send the requested signal on the D-Bus.")
            })
    }

    pub fn push_remove(
        &self,
        item: &Path<'static>,
        tree: &Tree<MTSync<TData>, TData>,
        interfaces: InterfacesRemoved,
    ) {
        let mut paths = Vec::new();
        for opath in tree.iter().filter(|opath| {
            opath
                .get_data()
                .as_ref()
                .map_or(false, |op_cxt| op_cxt.parent == *item)
        }) {
            paths.push(opath.get_name().clone());
            if self
                .removed_object_signal(
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
                )
                .is_err()
            {
                warn!("Signal on object removal was not sent to the D-Bus client");
            };
        }
        paths.push(item.clone());
        if self
            .removed_object_signal(item.clone(), interfaces)
            .is_err()
        {
            warn!("Signal on object removal was not sent to the D-Bus client");
        };

        if let Err(e) = block_on(self.sender.send(DbusAction::Remove(paths))) {
            warn!(
                "D-Bus remove event could not be sent to the processing thread; the D-Bus \
                server will still expect the D-Bus object with path {} to be present: {}",
                item, e,
            )
        }
    }

    /// Send an InterfacesRemoved signal on the D-Bus
    fn removed_object_signal(
        &self,
        object: Path<'static>,
        interfaces: InterfacesRemoved,
    ) -> Result<(), dbus::Error> {
        self.connection
            .send(
                ObjectManagerInterfacesRemoved { object, interfaces }
                    .to_emit_message(&Path::from(consts::STRATIS_BASE_PATH)),
            )
            .map(|_| ())
            .map_err(|_| {
                dbus::Error::new_failed("Failed to send the requested signal on the D-Bus.")
            })
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

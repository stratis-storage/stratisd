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
    blocking::stdintf::org_freedesktop_dbus::PropertiesPropertiesChanged,
    channel::Sender,
    message::SignalArgs,
    nonblock::SyncConnection,
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
    Add(ObjectPath<MTSync<TData>, TData>, InterfacesAdded),
    Remove(Path<'static>, InterfacesRemoved),
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

    /// Generates a new id for object paths.
    /// It is assumed that, while Stratisd is running, it will never generate
    /// more than 2^64 object paths. If it turns out that this is a bad
    /// assumption, the solution is to use unbounded integers.
    pub fn get_next_id(&self) -> u64 {
        self.next_index.fetch_add(1, Ordering::Relaxed)
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

    pub fn push_remove(&self, item: &Path<'static>, interfaces: InterfacesRemoved) {
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

    /// Send changed signal for Name property and invalidated signal for
    /// Devnode property.
    pub fn push_filesystem_name_change(&self, item: &Path<'static>, new_name: &str) {
        let mut changed = HashMap::new();
        changed.insert(
            consts::FILESYSTEM_NAME_PROP.into(),
            Variant(new_name.to_string().box_clone()),
        );

        if self
            .property_changed_invalidated_signal(
                &item,
                changed,
                vec![consts::FILESYSTEM_DEVNODE_PROP.into()],
                &consts::standard_filesystem_interfaces(),
            )
            .is_err()
        {
            warn!("Signal on filesystem name change was not sent to the D-Bus client");
        }
    }

    /// Send changed signal for pool Name property and invalidated signal for
    /// all Devnode properties of child filesystems.
    pub fn push_pool_name_change(
        &self,
        item: &Path<'static>,
        new_name: &str,
        tree: &Tree<MTSync<TData>, TData>,
    ) {
        let mut changed = HashMap::new();
        changed.insert(
            consts::POOL_NAME_PROP.into(),
            Variant(new_name.to_string().box_clone()),
        );

        if self
            .property_changed_invalidated_signal(
                &item,
                changed,
                vec![],
                &consts::standard_pool_interfaces(),
            )
            .is_err()
        {
            warn!("Signal on pool name change was not sent to the D-Bus client");
        }

        for opath in tree.iter().filter(|opath| {
            opath
                .get_data()
                .as_ref()
                .map_or(false, |op_cxt| op_cxt.parent == *item)
        }) {
            if let StratisUuid::Fs(_) = opath
                .get_data()
                .as_ref()
                .expect("all objects with parents have data")
                .uuid
            {
                if self
                    .property_changed_invalidated_signal(
                        &opath.get_name().clone(),
                        HashMap::new(),
                        vec![consts::FILESYSTEM_DEVNODE_PROP.into()],
                        &consts::standard_filesystem_interfaces(),
                    )
                    .is_err()
                {
                    warn!("Signal on filesystem devnode change was not sent to the D-Bus client");
                }
            }
        }
    }

    fn property_changed_invalidated_signal(
        &self,
        object: &Path,
        changed_properties: HashMap<String, Variant<Box<dyn RefArg>>>,
        invalidated_properties: Vec<String>,
        interfaces: &[String],
    ) -> Result<(), dbus::Error> {
        let mut prop_changed = PropertiesPropertiesChanged {
            changed_properties,
            invalidated_properties,
            interface_name: "temp_value".into(),
        };

        interfaces.iter().try_for_each(|interface| {
            prop_changed.interface_name = interface.to_owned();
            self.connection
                .send(prop_changed.to_emit_message(object))
                .map(|_| ())
                .map_err(|_| {
                    dbus::Error::new_failed("Failed to send the requested signal on the D-Bus.")
                })
        })
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

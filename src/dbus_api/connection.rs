// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use dbus::{
    arg::{RefArg, Variant},
    blocking::SyncConnection,
    channel::{MatchingReceiver, Sender},
    ffidisp::stdintf::org_freedesktop_dbus::{
        ObjectManagerInterfacesAdded, ObjectManagerInterfacesRemoved, PropertiesPropertiesChanged,
    },
    message::{MatchRule, SignalArgs},
    strings::Path,
    tree::{MTSync, Tree},
};
use futures::executor::block_on;
use tokio::sync::{mpsc::Receiver, RwLock};

use crate::{
    dbus_api::{
        consts,
        types::{DbusAction, InterfacesAdded, InterfacesRemoved, TData},
    },
    stratis::{StratisError, StratisResult},
};

/// Handler for a D-Bus tree.
/// Processes actions that require mutating the tree.
pub struct DbusTreeHandler {
    pub(super) connection: Arc<SyncConnection>,
    pub(super) tree: Arc<RwLock<Tree<MTSync<TData>, TData>>>,
    pub(super) receiver: Receiver<DbusAction>,
}

impl DbusTreeHandler {
    /// Process a D-Bus action (add/remove) request.
    pub async fn process_dbus_actions(&mut self) -> StratisResult<()> {
        loop {
            let action = self.receiver.recv().await.ok_or_else(|| {
                StratisError::Error(
                    "The channel from the D-Bus request handler to the D-Bus object handler was closed".to_string()
                )
            })?;
            let mut write_lock = self.tree.write().await;
            match action {
                DbusAction::Add(path, interfaces) => {
                    let path_name = path.get_name().clone();
                    write_lock.insert(path);
                    self.added_object_signal(path_name, interfaces)?;
                }
                DbusAction::ChangeFilesystemName(path, new_name) => {
                    self.property_changed_signal(
                        consts::FILESYSTEM_NAME_PROP,
                        &new_name,
                        &path,
                        &consts::standard_filesystem_interfaces(),
                    )?;
                }
                DbusAction::ChangePoolName(path, new_name) => {
                    self.property_changed_signal(
                        consts::POOL_NAME_PROP,
                        &new_name,
                        &path,
                        &consts::standard_pool_interfaces(),
                    )?;
                }
                DbusAction::InvalidateFilesystemDevnode(path) => {
                    self.property_invalidated_signal(
                        consts::FILESYSTEM_DEVNODE_PROP,
                        &path,
                        &consts::standard_filesystem_interfaces(),
                    )?;
                }
                DbusAction::Remove(path, interfaces) => {
                    write_lock.remove(&path);
                    self.removed_object_signal(path, interfaces)?;
                }
            }
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

    fn property_changed_signal<T: RefArg>(
        &self,
        prop_name: &str,
        new_value: &T,
        object: &Path,
        interfaces: &[String],
    ) -> Result<(), dbus::Error> {
        let mut prop_changed: PropertiesPropertiesChanged = Default::default();
        prop_changed
            .changed_properties
            .insert(prop_name.into(), Variant(new_value.box_clone()));

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

    fn property_invalidated_signal(
        &self,
        prop_name: &str,
        object: &Path,
        interfaces: &[String],
    ) -> Result<(), dbus::Error> {
        let mut prop_changed: PropertiesPropertiesChanged = Default::default();
        prop_changed.invalidated_properties.push(prop_name.into());

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
}

/// Handler for a D-Bus receiving connection.
/// stratisd has exactly one connection handler.
pub struct DbusConnectionHandler {
    connection: Arc<SyncConnection>,
    /// Shared boolean value, true if the engine should exit, otherwise false.
    should_exit: Arc<AtomicBool>,
}

impl DbusConnectionHandler {
    /// Set up the callback for the D-Bus connection.
    /// The callback is triggered on every D-Bus method call.
    /// Every method call requires a read lock on the D-Bus tree.
    /// All arguments are moved into the callback.
    pub(super) fn new(
        connection: Arc<SyncConnection>,
        tree: Arc<RwLock<Tree<MTSync<TData>, TData>>>,
        should_exit: Arc<AtomicBool>,
    ) -> DbusConnectionHandler {
        connection.start_receive(
            MatchRule::new_method_call(),
            Box::new(move |msg, conn_ref| {
                let read_lock = block_on(tree.read());
                let messages = read_lock.handle(&msg);
                if let Some(msgs) = messages {
                    for message in msgs {
                        if conn_ref.send(message).is_err() {
                            warn!("Failed to send response on the D-Bus");
                        }
                    }
                }
                true
            }),
        );
        DbusConnectionHandler {
            connection,
            should_exit,
        }
    }

    /// Handle a D-Bus action passed from a D-Bus connection.
    /// Returns true if stratisd should exit after handling this D-Bus
    /// request, otherwise false.
    /// This method causes the callback registered in the new() method to be
    /// invoked.
    pub fn process_dbus_request(&self) -> StratisResult<bool> {
        self.connection
            .process(Duration::from_millis(0))
            .map(|_| ())?;

        Ok(self.should_exit.load(Ordering::Relaxed))
    }
}

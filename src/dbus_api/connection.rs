// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::HashMap,
    sync::{mpsc::Receiver, Arc, RwLock},
    time::Duration,
};

use dbus::{
    arg::{RefArg, Variant},
    blocking::SyncConnection,
    channel::{MatchingReceiver, Sender},
    ffidisp::stdintf::org_freedesktop_dbus::{
        ObjectManagerInterfacesAdded, ObjectManagerInterfacesRemoved,
    },
    message::{MatchRule, SignalArgs},
    strings::Path,
    tree::{MTSync, Tree},
};

use crate::{
    dbus_api::{
        consts,
        types::{DbusAction, InterfacesAdded, InterfacesRemoved, TData},
    },
    stratis::StratisResult,
};

/// Handler for a D-Bus tree. This will be used to process add and remove requests from
/// the tree.
pub struct DbusTreeHandler {
    pub(super) connection: Arc<SyncConnection>,
    pub(super) tree: Arc<RwLock<Tree<MTSync<TData>, TData>>>,
    pub(super) receiver: Receiver<DbusAction>,
}

impl DbusTreeHandler {
    /// Process a D-Bus action (add/remove) request.
    pub fn process_dbus_action(&mut self) -> StratisResult<()> {
        let action = self.receiver.recv()?;
        match action {
            DbusAction::Add(path, interfaces) => {
                let path_name = path.get_name().clone();
                {
                    let mut rwlock = self.tree.write()?;
                    (*rwlock).insert(path);
                }
                self.added_object_signal(path_name, interfaces)?;
            }
            DbusAction::Remove(path, interfaces) => {
                {
                    let mut rwlock = self.tree.write()?;
                    (*rwlock).remove(&path);
                }
                self.removed_object_signal(path, interfaces)?;
            }
        }
        Ok(())
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
pub struct DbusConnectionHandler {
    connection: Arc<SyncConnection>,
}

impl DbusConnectionHandler {
    pub(super) fn new(
        connection: Arc<SyncConnection>,
        tree: Arc<RwLock<Tree<MTSync<TData>, TData>>>,
    ) -> DbusConnectionHandler {
        connection.start_receive(
            MatchRule::new_method_call(),
            Box::new(move |msg, conn_ref| {
                let messages = match tree.read() {
                    Ok(read_lock) => (*read_lock).handle(&msg),
                    Err(e) => {
                        error!(
                            "Failed to acquire read lock on D-Bus tree: {}; no more \
                            D-Bus requests will be handled",
                            e,
                        );
                        return false;
                    }
                };
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
        DbusConnectionHandler { connection }
    }

    /// Create a new reference to the D-Bus connection.
    pub fn new_connection_ref(&self) -> Arc<SyncConnection> {
        Arc::clone(&self.connection)
    }

    /// Handle a D-Bus action passed from the udev handler.
    pub fn process_dbus_request(&self) -> StratisResult<()> {
        Ok(self
            .connection
            .process(Duration::from_secs(5))
            .map(|_| ())?)
    }
}

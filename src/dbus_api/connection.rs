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

use async_std::task::block_on;
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
use tokio::sync::{mpsc::Receiver, Mutex};

use crate::{
    dbus_api::{
        consts,
        types::{DbusAction, InterfacesAdded, InterfacesRemoved, TData},
    },
    stratis::{StratisError, StratisResult},
};

/// Handler for a D-Bus tree. This will be used to process add and remove requests from
/// the tree.
pub struct DbusTreeHandler {
    pub(super) connection: Arc<SyncConnection>,
    pub(super) tree: Arc<Mutex<Tree<MTSync<TData>, TData>>>,
    pub(super) receiver: Receiver<DbusAction>,
}

impl DbusTreeHandler {
    /// Process a D-Bus action (add/remove) request.
    pub async fn process_dbus_action(&mut self) -> StratisResult<()> {
        let action = self.receiver.recv().await.ok_or_else(|| {
            StratisError::Error(
                "The channel from the D-Bus request handler to the D-Bus object handler was closed"
                    .to_string(),
            )
        })?;
        match action {
            DbusAction::Add(path, interfaces) => {
                let path_name = path.get_name().clone();
                {
                    let mut mutex = self.tree.lock().await;
                    mutex.insert(path);
                }
                self.added_object_signal(path_name, interfaces)?;
            }
            DbusAction::Remove(path, interfaces) => {
                {
                    let mut mutex = self.tree.lock().await;
                    mutex.remove(&path);
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
    should_exit: Arc<AtomicBool>,
}

impl DbusConnectionHandler {
    pub(super) fn new(
        connection: Arc<SyncConnection>,
        tree: Arc<Mutex<Tree<MTSync<TData>, TData>>>,
        should_exit: Arc<AtomicBool>,
    ) -> DbusConnectionHandler {
        connection.start_receive(
            MatchRule::new_method_call(),
            Box::new(move |msg, conn_ref| {
                let mutex = block_on(tree.lock());
                let messages = mutex.handle(&msg);
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

    /// Create a new reference to the D-Bus connection.
    pub fn new_connection_ref(&self) -> Arc<SyncConnection> {
        Arc::clone(&self.connection)
    }

    /// Handle a D-Bus action passed from the udev handler.
    pub fn process_dbus_request(&self) -> StratisResult<bool> {
        self.connection
            .process(Duration::from_millis(100))
            .map(|_| ())?;
        Ok(self.should_exit.load(Ordering::Relaxed))
    }
}

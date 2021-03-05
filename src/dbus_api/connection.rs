// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, sync::Arc};

use dbus::{
    arg::{RefArg, Variant},
    channel::{default_reply, Sender},
    message::{MatchRule, SignalArgs},
    nonblock::{
        stdintf::org_freedesktop_dbus::{
            ObjectManagerInterfacesAdded, ObjectManagerInterfacesRemoved,
            PropertiesPropertiesChanged,
        },
        SyncConnection,
    },
    Path,
};
use dbus_tree::{MTSync, Tree};
use futures::{executor::block_on, StreamExt};
use tokio::{
    sync::{mpsc::UnboundedReceiver, RwLock},
    task::spawn_blocking,
};

use crate::{
    dbus_api::{
        consts,
        types::{DbusAction, InterfacesAdded, InterfacesRemoved, TData},
    },
    engine::StratisUuid,
    stratis::{StratisError, StratisResult},
};

/// Handler for a D-Bus tree.
/// Proceses messages specifying tree mutations.
pub struct DbusTreeHandler {
    tree: Arc<RwLock<Tree<MTSync<TData>, TData>>>,
    receiver: UnboundedReceiver<DbusAction>,
    connection: Arc<SyncConnection>,
}

impl DbusTreeHandler {
    pub fn new(
        tree: Arc<RwLock<Tree<MTSync<TData>, TData>>>,
        receiver: UnboundedReceiver<DbusAction>,
        connection: Arc<SyncConnection>,
    ) -> Self {
        DbusTreeHandler {
            tree,
            receiver,
            connection,
        }
    }

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
                    if self.added_object_signal(path_name, interfaces).is_err() {
                        warn!("Signal on object add was not sent to the D-Bus client");
                    }
                }
                DbusAction::Remove(path, interfaces) => {
                    let paths = write_lock
                        .iter()
                        .filter_map(|opath| {
                            opath.get_data().as_ref().and_then(|op_cxt| {
                                if op_cxt.parent == path {
                                    Some((
                                        opath.get_name().clone(),
                                        match op_cxt.uuid {
                                            StratisUuid::Pool(_) => consts::pool_interface_list(),
                                            StratisUuid::Fs(_) => {
                                                consts::filesystem_interface_list()
                                            }
                                            StratisUuid::Dev(_) => {
                                                consts::blockdev_interface_list()
                                            }
                                        },
                                    ))
                                } else {
                                    None
                                }
                            })
                        })
                        .collect::<Vec<_>>();
                    for (path, interfaces) in paths {
                        write_lock.remove(&path);
                        if self.removed_object_signal(path, interfaces).is_err() {
                            warn!("Signal on object removal was not sent to the D-Bus client");
                        };
                    }
                    write_lock.remove(&path);
                    if self
                        .removed_object_signal(path.clone(), interfaces)
                        .is_err()
                    {
                        warn!("Signal on object removal was not sent to the D-Bus client");
                    };
                }
                DbusAction::FsNameChange(item, new_name) => {
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
                DbusAction::PoolNameChange(item, new_name) => {
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

                    for opath in write_lock.iter().filter(|opath| {
                        opath
                            .get_data()
                            .as_ref()
                            .map_or(false, |op_cxt| op_cxt.parent == item)
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

/// Handler for a D-Bus receiving connection.
/// stratisd has exactly one connection handler, but this handler spawns
/// a thread for every D-Bus method.
pub struct DbusConnectionHandler {
    connection: Arc<SyncConnection>,
    tree: Arc<RwLock<Tree<MTSync<TData>, TData>>>,
}

impl DbusConnectionHandler {
    pub(super) fn new(
        connection: Arc<SyncConnection>,
        tree: Arc<RwLock<Tree<MTSync<TData>, TData>>>,
    ) -> DbusConnectionHandler {
        DbusConnectionHandler { connection, tree }
    }

    /// Handle a D-Bus action passed from a D-Bus connection.
    /// Spawn a new thread for every D-Bus method call.
    /// Every method call requires a read lock on the D-Bus tree.
    pub async fn process_dbus_requests(&self) -> StratisResult<()> {
        let match_msg = self
            .connection
            .add_match(MatchRule::new_method_call())
            .await?;
        let (_match_msg, mut stream) = match_msg.msg_stream();
        while let Some(msg) = stream.next().await {
            let tree = Arc::clone(&self.tree);
            let connection = Arc::clone(&self.connection);
            spawn_blocking(move || {
                let lock = block_on(tree.read());
                if let Some(msgs) = lock.handle(&msg) {
                    for msg in msgs {
                        if connection.send(msg).is_err() {
                            warn!("Failed to send reply to D-Bus client");
                        }
                    }
                } else {
                    let reply = default_reply(&msg);
                    if let Some(r) = reply {
                        if connection.send(r).is_err() {
                            warn!("Failed to send reply to D-Bus client");
                        }
                    }
                }
            });
        }
        Ok(())
    }
}

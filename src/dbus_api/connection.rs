// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, sync::Arc, time::Duration};

use dbus::{
    arg::{RefArg, Variant},
    blocking::{
        stdintf::org_freedesktop_dbus::{
            ObjectManagerInterfacesAdded, ObjectManagerInterfacesRemoved,
            PropertiesPropertiesChanged,
        },
        SyncConnection,
    },
    channel::{default_reply, MatchingReceiver, Sender},
    message::{MatchRule, SignalArgs},
    Path,
};
use futures::{
    executor::block_on,
    future::{select, Either},
    pin_mut,
};
use tokio::{
    sync::{
        broadcast::{error::TryRecvError, Receiver},
        mpsc::UnboundedReceiver,
    },
    task::spawn_blocking,
};

use crate::{
    dbus_api::{
        consts,
        types::{DbusAction, InterfacesAddedThreadSafe, InterfacesRemoved, LockableTree},
        util::thread_safe_to_dbus_sendable,
    },
    engine::StratisUuid,
    stratis::{StratisError, StratisResult},
};

/// Handler for a D-Bus tree.
/// Proceses messages specifying tree mutations.
pub struct DbusTreeHandler {
    tree: LockableTree,
    receiver: UnboundedReceiver<DbusAction>,
    connection: Arc<SyncConnection>,
    should_exit: Receiver<bool>,
}

impl DbusTreeHandler {
    pub fn new(
        tree: LockableTree,
        receiver: UnboundedReceiver<DbusAction>,
        connection: Arc<SyncConnection>,
        should_exit: Receiver<bool>,
    ) -> Self {
        DbusTreeHandler {
            tree,
            receiver,
            connection,
            should_exit,
        }
    }

    /// Process a D-Bus action (add/remove) request.
    pub fn process_dbus_actions(&mut self) -> StratisResult<()> {
        loop {
            let (action, mut write_lock) = {
                let recv_fut = self.receiver.recv();
                let should_exit_fut = self.should_exit.recv();

                pin_mut!(recv_fut);
                pin_mut!(should_exit_fut);

                let (action, should_exit_fut) = match block_on(select(recv_fut, should_exit_fut)) {
                    Either::Left((a, se)) => {
                        (
                            a.ok_or_else(|| {
                                StratisError::Error(
                                    "The channel from the D-Bus request handler to the D-Bus object handler was closed".to_string()
                                )
                            })?,
                            se
                        )
                    },
                    Either::Right((Ok(true), _)) => {
                        info!("D-Bus tree handler was notified to exit");
                        break;
                    },
                    Either::Right((Err(_), _)) => {
                        warn!("D-Bus tree handler can no longer be notified to exit; shutting down...");
                        break;
                    },
                    _ => continue,
                };

                let write_fut = self.tree.write();

                pin_mut!(write_fut);

                let write_lock = match block_on(select(write_fut, should_exit_fut)) {
                    Either::Left((wl, _)) => wl,
                    Either::Right((Ok(true), _)) => {
                        info!("D-Bus tree handler was notified to exit");
                        break;
                    }
                    Either::Right((Err(_), _)) => {
                        warn!("D-Bus tree handler can no longer be notified to exit; shutting down...");
                        break;
                    }
                    _ => continue,
                };

                (action, write_lock)
            };

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
        Ok(())
    }

    /// Send an InterfacesAdded signal on the D-Bus
    fn added_object_signal(
        &self,
        object: Path<'static>,
        interfaces: InterfacesAddedThreadSafe,
    ) -> Result<(), dbus::Error> {
        self.connection
            .send(
                ObjectManagerInterfacesAdded {
                    object,
                    interfaces: thread_safe_to_dbus_sendable(interfaces),
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
    tree: LockableTree,
    should_exit: Receiver<bool>,
}

impl DbusConnectionHandler {
    pub(super) fn new(
        connection: Arc<SyncConnection>,
        tree: LockableTree,
        should_exit: Receiver<bool>,
    ) -> DbusConnectionHandler {
        DbusConnectionHandler {
            connection,
            tree,
            should_exit,
        }
    }

    /// Handle a D-Bus action passed from a D-Bus connection.
    /// Spawn a new thread for every D-Bus method call.
    /// Every method call requires a read lock on the D-Bus tree.
    pub fn process_dbus_requests(&mut self) -> StratisResult<()> {
        let tree = self.tree.clone();
        let connection = Arc::clone(&self.connection);
        let _ = self.connection.start_receive(
            MatchRule::new_method_call(),
            Box::new(move |msg, _| {
                let cloned_tree = tree.clone();
                let cloned_connection = Arc::clone(&connection);
                spawn_blocking(move || {
                    let lock = cloned_tree.blocking_read();
                    if let Some(msgs) = lock.handle(&msg) {
                        for msg in msgs {
                            if cloned_connection.send(msg).is_err() {
                                warn!("Failed to send reply to D-Bus client");
                            }
                        }
                    } else {
                        let reply = default_reply(&msg);
                        if let Some(r) = reply {
                            if cloned_connection.send(r).is_err() {
                                warn!("Failed to send reply to D-Bus client");
                            }
                        }
                    }
                });
                true
            }),
        );
        loop {
            self.connection.process(Duration::from_millis(100))?;
            match self.should_exit.try_recv() {
                Ok(true) => {
                    info!("D-Bus connection handler thread notified to exit");
                    break;
                }
                Err(TryRecvError::Lagged(_)) | Err(TryRecvError::Closed) => {
                    warn!("D-Bus connection handler can't be notified to exit. Shutting down...");
                    break;
                }
                _ => (),
            }
        }

        Ok(())
    }
}

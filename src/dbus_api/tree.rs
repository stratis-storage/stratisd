// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, sync::Arc};

use dbus::{
    arg::{RefArg, Variant},
    blocking::{
        stdintf::org_freedesktop_dbus::{
            ObjectManagerInterfacesAdded, ObjectManagerInterfacesRemoved,
            PropertiesPropertiesChanged,
        },
        SyncConnection,
    },
    channel::Sender,
    message::SignalArgs,
    Path,
};
use dbus_tree::{MTSync, ObjectPath};
use tokio::sync::{broadcast::Receiver, mpsc::UnboundedReceiver};

use crate::{
    dbus_api::{
        consts,
        types::{
            DbusAction, InterfacesAddedThreadSafe, InterfacesRemoved, LockableTree, TData,
            TreeReadLock, TreeWriteLock,
        },
        util::{poll_exit_and_future, thread_safe_to_dbus_sendable},
    },
    engine::{ActionAvailability, Engine, StratisUuid},
    stratis::{StratisError, StratisResult},
};

/// Handler for a D-Bus tree.
/// Processes messages specifying tree mutations or traversals.
pub struct DbusTreeHandler<E> {
    tree: LockableTree<E>,
    receiver: UnboundedReceiver<DbusAction<E>>,
    connection: Arc<SyncConnection>,
    should_exit: Receiver<()>,
}

impl<E> DbusTreeHandler<E>
where
    E: Engine,
{
    pub fn new(
        tree: LockableTree<E>,
        receiver: UnboundedReceiver<DbusAction<E>>,
        connection: Arc<SyncConnection>,
        should_exit: Receiver<()>,
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
        while let Some(output) =
            poll_exit_and_future(self.should_exit.recv(), self.receiver.recv())?
        {
            let action = output.ok_or_else(|| {
                StratisError::Msg(
                    "The channel from the D-Bus request handler to the D-Bus object handler was closed".to_string()
                )
            })?;

            if self.handle_dbus_action(action)?.is_none() {
                break;
            }
        }
        Ok(())
    }

    /// Handle an object path being added to the tree of D-Bus objects.
    fn handle_add_action(
        &self,
        mut write_lock: TreeWriteLock<E>,
        path: ObjectPath<MTSync<TData<E>>, TData<E>>,
        interfaces: InterfacesAddedThreadSafe,
    ) {
        let path_name = path.get_name().clone();
        write_lock.insert(path);
        if self.added_object_signal(path_name, interfaces).is_err() {
            warn!("Signal on object add was not sent to the D-Bus client");
        }
    }

    /// Handle an object path being removed from the tree of D-Bus objects.
    fn handle_remove_action(
        &self,
        mut write_lock: TreeWriteLock<E>,
        path: Path<'static>,
        interfaces: InterfacesRemoved,
    ) {
        let paths = write_lock
            .iter()
            .filter_map(|opath| {
                opath.get_data().as_ref().and_then(|op_cxt| {
                    if op_cxt.parent == path {
                        Some((
                            opath.get_name().clone(),
                            match op_cxt.uuid {
                                StratisUuid::Pool(_) => consts::pool_interface_list(),
                                StratisUuid::Fs(_) => consts::filesystem_interface_list(),
                                StratisUuid::Dev(_) => consts::blockdev_interface_list(),
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

    /// Handle a filesystem name change in the engine.
    fn handle_fs_name_change(&self, item: Path<'static>, new_name: String) {
        let mut changed = HashMap::new();
        changed.insert(
            consts::FILESYSTEM_NAME_PROP.into(),
            Variant(new_name.box_clone()),
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

    /// Handle a pool name change in the engine.
    fn handle_pool_name_change(
        &self,
        read_lock: TreeReadLock<E>,
        item: Path<'static>,
        new_name: String,
    ) {
        let mut changed = HashMap::new();
        changed.insert(consts::POOL_NAME_PROP.into(), Variant(new_name.box_clone()));

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

        for opath in read_lock.iter().filter(|opath| {
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

    /// Handle a change of available actions for a pool in the engine.
    fn handle_pool_avail_actions_change(
        &self,
        item: Path<'static>,
        new_avail_actions: ActionAvailability,
    ) {
        let mut changed = HashMap::new();
        changed.insert(
            consts::POOL_AVAIL_ACTIONS_PROP.into(),
            Variant(Box::new(new_avail_actions.to_string()) as Box<(dyn RefArg + 'static)>),
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
            warn!("Signal on pool available actions mode change was not sent to the D-Bus client");
        }
    }

    /// Handle a D-Bus action that has been generated by the connection processing
    /// handle.
    fn handle_dbus_action(&mut self, action: DbusAction<E>) -> StratisResult<Option<()>> {
        match action {
            DbusAction::Add(path, interfaces) => {
                if let Some(write_lock) =
                    poll_exit_and_future(self.should_exit.recv(), self.tree.write())?
                {
                    self.handle_add_action(write_lock, path, interfaces);
                    Ok(Some(()))
                } else {
                    Ok(None)
                }
            }
            DbusAction::Remove(path, interfaces) => {
                if let Some(write_lock) =
                    poll_exit_and_future(self.should_exit.recv(), self.tree.write())?
                {
                    self.handle_remove_action(write_lock, path, interfaces);
                    Ok(Some(()))
                } else {
                    Ok(None)
                }
            }
            DbusAction::FsNameChange(item, new_name) => {
                self.handle_fs_name_change(item, new_name);
                Ok(Some(()))
            }
            DbusAction::PoolNameChange(item, new_name) => {
                if let Some(read_lock) =
                    poll_exit_and_future(self.should_exit.recv(), self.tree.read())?
                {
                    self.handle_pool_name_change(read_lock, item, new_name);
                    Ok(Some(()))
                } else {
                    Ok(None)
                }
            }
            DbusAction::PoolAvailActions(item, new_avail_actions) => {
                self.handle_pool_avail_actions_change(item, new_avail_actions);
                Ok(Some(()))
            }
            DbusAction::FsSizeChange(uuid, new_size) => {
                // TODO: Add filesystem property handling
                info!(
                    "Filesystem with UUID {} changed size to {} bytes",
                    uuid, *new_size
                );
                Ok(Some(()))
            }
        }
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

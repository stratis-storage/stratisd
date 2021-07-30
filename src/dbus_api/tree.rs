// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, iter::once, sync::Arc};

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

use devicemapper::Bytes;

use crate::{
    dbus_api::{
        api::prop_conv::locked_pools_to_prop,
        consts,
        filesystem::prop_conv::{fs_size_to_prop, fs_used_to_prop},
        pool::prop_conv::{
            avail_actions_to_prop, clevis_info_to_prop, key_desc_to_prop, pool_alloc_to_prop,
            pool_size_to_prop, pool_used_to_prop,
        },
        types::{
            DbusAction, InterfacesAddedThreadSafe, InterfacesRemoved, LockableTree, TData,
            TreeReadLock, TreeWriteLock,
        },
        util::{poll_exit_and_future, thread_safe_to_dbus_sendable},
    },
    engine::{
        ActionAvailability, Engine, FilesystemUuid, LockedPoolInfo, PoolEncryptionInfo, PoolUuid,
        StratisUuid,
    },
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

            if !self.handle_dbus_action(action)? {
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
            box_variant!(avail_actions_to_prop(new_avail_actions)),
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

    /// Handle a change of the key description for a pool in the engine.
    fn handle_pool_key_desc_change(&self, item: Path<'static>, ei: Option<PoolEncryptionInfo>) {
        let mut changed = HashMap::new();
        changed.insert(
            consts::POOL_KEY_DESC_PROP.into(),
            box_variant!(key_desc_to_prop(ei)),
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
            warn!("Signal on pool key description change was not sent to the D-Bus client");
        }
    }

    /// Handle a change of the key description for a pool in the engine.
    fn handle_pool_clevis_info_change(&self, item: Path<'static>, ei: Option<PoolEncryptionInfo>) {
        let mut changed = HashMap::new();
        changed.insert(
            consts::POOL_CLEVIS_INFO_PROP.into(),
            box_variant!(clevis_info_to_prop(ei)),
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
            warn!("Signal on pool Clevis information change was not sent to the D-Bus client");
        }
    }

    /// Handle a change of available actions for a pool in the engine.
    fn handle_pool_cache_change(&self, item: Path<'static>, b: bool) {
        let mut changed = HashMap::new();
        changed.insert(consts::POOL_HAS_CACHE_PROP.into(), box_variant!(b));

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

    /// Handle a change of locked pools registered in the engine.
    fn handle_locked_pools_change(&self, locked_pools: HashMap<PoolUuid, LockedPoolInfo>) {
        let mut changed = HashMap::new();
        changed.insert(
            consts::LOCKED_POOLS_PROP.into(),
            box_variant!(locked_pools_to_prop(locked_pools)),
        );

        if self
            .property_changed_invalidated_signal(
                &Path::new(consts::STRATIS_BASE_PATH).expect("Valid path"),
                changed,
                vec![],
                &consts::standard_pool_interfaces(),
            )
            .is_err()
        {
            warn!("Signal on pool available actions mode change was not sent to the D-Bus client");
        }
    }

    /// Look up the filesystem path of the filesystem and notify clients of any
    /// changes to properties that change in the background.
    #[allow(clippy::option_option)]
    fn handle_fs_background_change(
        &self,
        read_lock: TreeReadLock<E>,
        uuid: FilesystemUuid,
        new_size: Option<Bytes>,
        new_used: Option<Option<Bytes>>,
    ) {
        if let Some(path) = read_lock
            .iter()
            .filter_map(|opath| {
                opath.get_data().as_ref().and_then(|data| {
                    if let StratisUuid::Fs(u) = data.uuid {
                        if u == uuid {
                            Some(opath.get_name())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
            })
            .next()
        {
            if let Err(e) = self.property_changed_invalidated_signal(
                path,
                vec![
                    (
                        consts::FILESYSTEM_SIZE_PROP.to_string(),
                        new_size.map(|s| box_variant!(fs_size_to_prop(s))),
                    ),
                    (
                        consts::FILESYSTEM_USED_PROP.to_string(),
                        new_used.map(|u| box_variant!(fs_used_to_prop(u))),
                    ),
                ]
                .into_iter()
                .filter_map(|(prop, opt)| opt.map(|val| (prop, val)))
                .collect::<HashMap<_, _>>(),
                vec![],
                &consts::standard_filesystem_interfaces(),
            ) {
                warn!(
                    "Failed to send a signal over D-Bus indicating filesystem size change: {}",
                    e
                );
            }
        } else {
            warn!("A filesystem size was changed in the engine but no filesystem with the corresponding UUID could be found in the D-Bus layer");
        }
    }

    /// Look up the pool path of the pool and send out a signal indicating which
    /// properties, have changed in the background.
    #[allow(clippy::option_option)]
    fn handle_pool_background_change(
        &self,
        read_lock: TreeReadLock<E>,
        uuid: PoolUuid,
        new_used: Option<Option<Bytes>>,
        new_alloc: Option<Bytes>,
    ) {
        if let Some(path) = read_lock
            .iter()
            .filter_map(|opath| {
                opath.get_data().as_ref().and_then(|data| {
                    if let StratisUuid::Pool(u) = data.uuid {
                        if u == uuid {
                            Some(opath.get_name())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
            })
            .next()
        {
            if let Err(e) = self.property_changed_invalidated_signal(
                path,
                vec![
                    (
                        consts::POOL_TOTAL_USED_PROP.to_string(),
                        new_used.map(|u| box_variant!(pool_used_to_prop(u))),
                    ),
                    (
                        consts::POOL_ALLOC_SIZE_PROP.to_string(),
                        new_alloc.map(|a| box_variant!(pool_alloc_to_prop(a))),
                    ),
                ]
                .into_iter()
                .filter_map(|(prop, opt)| opt.map(|val| (prop, val)))
                .collect::<HashMap<_, _>>(),
                vec![],
                &consts::standard_pool_interfaces(),
            ) {
                warn!(
                    "Failed to send a signal over D-Bus indicating pool usage change: {}",
                    e
                );
            }
        } else {
            warn!("A pool usage was changed in the engine but no pool with the corresponding UUID could be found in the D-Bus layer");
        }
    }

    /// Send a signal indicating that the pool total size has changed.
    fn handle_pool_size_change(&self, path: Path<'static>, new_size: Bytes) {
        if let Err(e) = self.property_changed_invalidated_signal(
            &path,
            once((
                consts::POOL_TOTAL_SIZE_PROP.to_string(),
                box_variant!(pool_size_to_prop(new_size)),
            ))
            .collect::<HashMap<_, _>>(),
            vec![],
            &consts::standard_pool_interfaces(),
        ) {
            warn!(
                "Failed to send a signal over D-Bus indicating pool size change: {}",
                e
            );
        }
    }

    /// Handle a D-Bus action that has been generated by the connection processing
    /// handle.
    fn handle_dbus_action(&mut self, action: DbusAction<E>) -> StratisResult<bool> {
        match action {
            DbusAction::Add(path, interfaces) => {
                if let Some(write_lock) =
                    poll_exit_and_future(self.should_exit.recv(), self.tree.write())?
                {
                    self.handle_add_action(write_lock, path, interfaces);
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            DbusAction::Remove(path, interfaces) => {
                if let Some(write_lock) =
                    poll_exit_and_future(self.should_exit.recv(), self.tree.write())?
                {
                    self.handle_remove_action(write_lock, path, interfaces);
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            DbusAction::FsNameChange(item, new_name) => {
                self.handle_fs_name_change(item, new_name);
                Ok(true)
            }
            DbusAction::PoolNameChange(item, new_name) => {
                if let Some(read_lock) =
                    poll_exit_and_future(self.should_exit.recv(), self.tree.read())?
                {
                    self.handle_pool_name_change(read_lock, item, new_name);
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            DbusAction::PoolAvailActions(item, new_avail_actions) => {
                self.handle_pool_avail_actions_change(item, new_avail_actions);
                Ok(true)
            }
            DbusAction::PoolKeyDescChange(item, ei) => {
                self.handle_pool_key_desc_change(item, ei);
                Ok(true)
            }
            DbusAction::PoolClevisInfoChange(item, ei) => {
                self.handle_pool_clevis_info_change(item, ei);
                Ok(true)
            }
            DbusAction::FsBackgroundChange(uuid, new_size, new_used) => {
                if let Some(read_lock) =
                    poll_exit_and_future(self.should_exit.recv(), self.tree.read())?
                {
                    self.handle_fs_background_change(read_lock, uuid, new_size, new_used);
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            DbusAction::PoolBackgroundChange(uuid, new_usage, new_alloc) => {
                if let Some(read_lock) =
                    poll_exit_and_future(self.should_exit.recv(), self.tree.read())?
                {
                    self.handle_pool_background_change(read_lock, uuid, new_usage, new_alloc);
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            DbusAction::PoolCacheChange(item, has_cache) => {
                self.handle_pool_cache_change(item, has_cache);
                Ok(true)
            }
            DbusAction::PoolSizeChange(path, new_size) => {
                self.handle_pool_size_change(path, new_size);
                Ok(true)
            }
            DbusAction::LockedPoolsChange(pools) => {
                self.handle_locked_pools_change(pools);
                Ok(true)
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
        object: &Path<'_>,
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

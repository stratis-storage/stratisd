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

use devicemapper::{Bytes, Sectors};

use crate::{
    dbus_api::{
        api::prop_conv::{locked_pools_to_prop, stopped_pools_to_prop},
        blockdev::prop_conv::{blockdev_new_size_to_prop, blockdev_user_info_to_prop},
        consts,
        filesystem::prop_conv::{fs_size_to_prop, fs_used_to_prop},
        pool::prop_conv::{
            avail_actions_to_prop, clevis_info_to_prop, key_desc_to_prop, pool_alloc_to_prop,
            pool_size_to_prop, pool_used_to_prop,
        },
        types::{
            DbusAction, InterfacesAddedThreadSafe, InterfacesRemoved, LockableTree, SignalChange,
            TData, TreeReadLock, TreeWriteLock,
        },
        util::{poll_exit_and_future, thread_safe_to_dbus_sendable},
    },
    engine::{
        ActionAvailability, DevUuid, Engine, FilesystemUuid, LockedPoolInfo, PoolEncryptionInfo,
        PoolUuid, StoppedPoolInfo, StratisUuid,
    },
    stratis::{StratisError, StratisResult},
};

type PropertySignal = HashMap<String, (HashMap<String, Variant<Box<dyn RefArg>>>, Vec<String>)>;

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
        if self
            .property_changed_invalidated_signal(
                &item,
                prop_hashmap! {
                    consts::FILESYSTEM_INTERFACE_NAME_3_0 => {
                        vec![consts::FILESYSTEM_DEVNODE_PROP.into()],
                        consts::FILESYSTEM_NAME_PROP.to_string() =>
                        Variant(new_name.box_clone())
                    },
                    consts::FILESYSTEM_INTERFACE_NAME_3_1 => {
                        vec![consts::FILESYSTEM_DEVNODE_PROP.into()],
                        consts::FILESYSTEM_NAME_PROP.to_string() =>
                        Variant(new_name.box_clone())
                    },
                    consts::FILESYSTEM_INTERFACE_NAME_3_2 => {
                        vec![consts::FILESYSTEM_DEVNODE_PROP.into()],
                        consts::FILESYSTEM_NAME_PROP.to_string() =>
                        Variant(new_name.box_clone())
                    },
                    consts::FILESYSTEM_INTERFACE_NAME_3_3 => {
                        vec![consts::FILESYSTEM_DEVNODE_PROP.into()],
                        consts::FILESYSTEM_NAME_PROP.to_string() =>
                        Variant(new_name.box_clone())
                    }
                },
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
        if self
            .property_changed_invalidated_signal(
                &item,
                prop_hashmap! {
                    consts::POOL_INTERFACE_NAME_3_0 => {
                        Vec::new(),
                        consts::POOL_NAME_PROP.to_string() =>
                        Variant(new_name.box_clone())
                    },
                    consts::POOL_INTERFACE_NAME_3_1 => {
                        Vec::new(),
                        consts::POOL_NAME_PROP.to_string() =>
                        Variant(new_name.box_clone())
                    },
                    consts::POOL_INTERFACE_NAME_3_2 => {
                        Vec::new(),
                        consts::POOL_NAME_PROP.to_string() =>
                        Variant(new_name.box_clone())
                    },
                    consts::POOL_INTERFACE_NAME_3_3 => {
                        Vec::new(),
                        consts::POOL_NAME_PROP.to_string() =>
                        Variant(new_name.box_clone())
                    }
                },
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
                        prop_hashmap! {
                            consts::FILESYSTEM_INTERFACE_NAME_3_0 => {
                                vec![consts::FILESYSTEM_DEVNODE_PROP.into()]
                            },
                            consts::FILESYSTEM_INTERFACE_NAME_3_1 => {
                                vec![consts::FILESYSTEM_DEVNODE_PROP.into()]
                            },
                            consts::FILESYSTEM_INTERFACE_NAME_3_2 => {
                                vec![consts::FILESYSTEM_DEVNODE_PROP.into()]
                            },
                            consts::FILESYSTEM_INTERFACE_NAME_3_3 => {
                                vec![consts::FILESYSTEM_DEVNODE_PROP.into()]
                            }
                        },
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
        let avail_prop = avail_actions_to_prop(new_avail_actions);
        if self
            .property_changed_invalidated_signal(
                &item,
                prop_hashmap! {
                    consts::POOL_INTERFACE_NAME_3_0 => {
                        Vec::new(),
                        consts::POOL_AVAIL_ACTIONS_PROP.to_string() =>
                        box_variant!(avail_prop.clone())
                    },
                    consts::POOL_INTERFACE_NAME_3_1 => {
                        Vec::new(),
                        consts::POOL_AVAIL_ACTIONS_PROP.to_string() =>
                        box_variant!(avail_prop.clone())
                    },
                    consts::POOL_INTERFACE_NAME_3_2 => {
                        Vec::new(),
                        consts::POOL_AVAIL_ACTIONS_PROP.to_string() =>
                        box_variant!(avail_prop.clone())
                    },
                    consts::POOL_INTERFACE_NAME_3_3 => {
                        Vec::new(),
                        consts::POOL_AVAIL_ACTIONS_PROP.to_string() =>
                        box_variant!(avail_prop)
                    }
                },
            )
            .is_err()
        {
            warn!("Signal on pool available actions mode change was not sent to the D-Bus client");
        }
    }

    /// Handle a change of the key description for a pool in the engine.
    fn handle_pool_key_desc_change(&self, item: Path<'static>, ei: Option<PoolEncryptionInfo>) {
        let kd_prop = key_desc_to_prop(ei);
        if self
            .property_changed_invalidated_signal(
                &item,
                prop_hashmap! {
                    consts::POOL_INTERFACE_NAME_3_0 => {
                        Vec::new(),
                        consts::POOL_KEY_DESC_PROP.to_string() =>
                        box_variant!(kd_prop.clone())
                    },
                    consts::POOL_INTERFACE_NAME_3_1 => {
                        Vec::new(),
                        consts::POOL_KEY_DESC_PROP.to_string() =>
                        box_variant!(kd_prop.clone())
                    },
                    consts::POOL_INTERFACE_NAME_3_2 => {
                        Vec::new(),
                        consts::POOL_KEY_DESC_PROP.to_string() =>
                        box_variant!(kd_prop.clone())
                    },
                    consts::POOL_INTERFACE_NAME_3_3 => {
                        Vec::new(),
                        consts::POOL_KEY_DESC_PROP.to_string() =>
                        box_variant!(kd_prop)
                    }
                },
            )
            .is_err()
        {
            warn!("Signal on pool key description change was not sent to the D-Bus client");
        }
    }

    /// Handle a change of the key description for a pool in the engine.
    fn handle_pool_clevis_info_change(&self, item: Path<'static>, ei: Option<PoolEncryptionInfo>) {
        let ci_prop = clevis_info_to_prop(ei);
        if self
            .property_changed_invalidated_signal(
                &item,
                prop_hashmap! {
                    consts::POOL_INTERFACE_NAME_3_0 => {
                        Vec::new(),
                        consts::POOL_CLEVIS_INFO_PROP.to_string() =>
                        box_variant!(ci_prop.clone())
                    },
                    consts::POOL_INTERFACE_NAME_3_1 => {
                        Vec::new(),
                        consts::POOL_CLEVIS_INFO_PROP.to_string() =>
                        box_variant!(ci_prop.clone())
                    },
                    consts::POOL_INTERFACE_NAME_3_2 => {
                        Vec::new(),
                        consts::POOL_CLEVIS_INFO_PROP.to_string() =>
                        box_variant!(ci_prop.clone())
                    },
                    consts::POOL_INTERFACE_NAME_3_3 => {
                        Vec::new(),
                        consts::POOL_CLEVIS_INFO_PROP.to_string() =>
                        box_variant!(ci_prop)
                    }
                },
            )
            .is_err()
        {
            warn!("Signal on pool Clevis information change was not sent to the D-Bus client");
        }
    }

    /// Handle a change of available actions for a pool in the engine.
    fn handle_pool_cache_change(&self, item: Path<'static>, b: bool) {
        if self
            .property_changed_invalidated_signal(
                &item,
                prop_hashmap! {
                    consts::POOL_INTERFACE_NAME_3_0 => {
                        Vec::new(),
                        consts::POOL_HAS_CACHE_PROP.to_string() => box_variant!(b)
                    },
                    consts::POOL_INTERFACE_NAME_3_1 => {
                        Vec::new(),
                        consts::POOL_HAS_CACHE_PROP.to_string() => box_variant!(b)
                    },
                    consts::POOL_INTERFACE_NAME_3_2 => {
                        Vec::new(),
                        consts::POOL_HAS_CACHE_PROP.to_string() => box_variant!(b)
                    },
                    consts::POOL_INTERFACE_NAME_3_3 => {
                        Vec::new(),
                        consts::POOL_HAS_CACHE_PROP.to_string() => box_variant!(b)
                    }
                },
            )
            .is_err()
        {
            warn!("Signal on pool available actions mode change was not sent to the D-Bus client");
        }
    }

    /// Handle a change of locked pools registered in the engine.
    fn handle_locked_pools_change(&self, locked_pools: HashMap<PoolUuid, LockedPoolInfo>) {
        if self
            .property_changed_invalidated_signal(
                &Path::new(consts::STRATIS_BASE_PATH).expect("Valid path"),
                prop_hashmap! {
                    consts::MANAGER_INTERFACE_NAME_3_0 => {
                        Vec::new(),
                        consts::LOCKED_POOLS_PROP.to_string() =>
                        box_variant!(locked_pools_to_prop(&locked_pools))
                    },
                    consts::MANAGER_INTERFACE_NAME_3_1 => {
                        Vec::new(),
                        consts::LOCKED_POOLS_PROP.to_string() =>
                        box_variant!(locked_pools_to_prop(&locked_pools))
                    }
                },
            )
            .is_err()
        {
            warn!("Signal on locked pools change was not sent to the D-Bus client");
        }
    }

    /// Handle a change of stopped pools registered in the engine.
    fn handle_stopped_pools_change(&self, stopped_pools: HashMap<PoolUuid, StoppedPoolInfo>) {
        if self
            .property_changed_invalidated_signal(
                &Path::new(consts::STRATIS_BASE_PATH).expect("Valid path"),
                prop_hashmap! {
                    consts::MANAGER_INTERFACE_NAME_3_2 => {
                        Vec::new(),
                        consts::STOPPED_POOLS_PROP.to_string() =>
                        box_variant!(stopped_pools_to_prop(&stopped_pools))
                    },
                    consts::MANAGER_INTERFACE_NAME_3_3 => {
                        Vec::new(),
                        consts::STOPPED_POOLS_PROP.to_string() =>
                        box_variant!(stopped_pools_to_prop(&stopped_pools))
                    }
                },
            )
            .is_err()
        {
            warn!("Signal on stopped pools change was not sent to the D-Bus client");
        }
    }

    /// Look up the filesystem path of the filesystem and notify clients of any
    /// changes to properties that change in the background.
    fn handle_fs_background_change(
        &self,
        read_lock: TreeReadLock<E>,
        uuid: FilesystemUuid,
        new_used: SignalChange<Option<Bytes>>,
        new_size: SignalChange<Bytes>,
    ) {
        handle_background_change!(
            self,
            read_lock,
            uuid,
            Fs,
            "filesystem",
            consts::FILESYSTEM_INTERFACE_NAME_3_0 => {
                consts::FILESYSTEM_USED_PROP.to_string(),
                fs_used_to_prop,
                new_used,
                consts::FILESYSTEM_SIZE_PROP.to_string(),
                fs_size_to_prop,
                new_size
            },
            consts::FILESYSTEM_INTERFACE_NAME_3_1 => {
                consts::FILESYSTEM_USED_PROP.to_string(),
                fs_used_to_prop,
                new_used,
                consts::FILESYSTEM_SIZE_PROP.to_string(),
                fs_size_to_prop,
                new_size
            },
            consts::FILESYSTEM_INTERFACE_NAME_3_2 => {
                consts::FILESYSTEM_USED_PROP.to_string(),
                fs_used_to_prop,
                new_used,
                consts::FILESYSTEM_SIZE_PROP.to_string(),
                fs_size_to_prop,
                new_size
            },
            consts::FILESYSTEM_INTERFACE_NAME_3_3 => {
                consts::FILESYSTEM_USED_PROP.to_string(),
                fs_used_to_prop,
                new_used,
                consts::FILESYSTEM_SIZE_PROP.to_string(),
                fs_size_to_prop,
                new_size
            }
        );
    }

    /// Look up the pool path of the pool and notify clients of any changes to
    /// properties that change in the background.
    fn handle_pool_background_change(
        &self,
        read_lock: TreeReadLock<E>,
        uuid: PoolUuid,
        new_used: SignalChange<Option<Bytes>>,
        new_alloc: SignalChange<Bytes>,
        new_no_space: SignalChange<bool>,
    ) {
        handle_background_change!(
            self,
            read_lock,
            uuid,
            Pool,
            "pool",
            consts::POOL_INTERFACE_NAME_3_0 => {
                consts::POOL_TOTAL_USED_PROP.to_string(),
                pool_used_to_prop,
                new_used,
                consts::POOL_ALLOC_SIZE_PROP.to_string(),
                pool_alloc_to_prop,
                new_alloc
            },
            consts::POOL_INTERFACE_NAME_3_1 => {
                consts::POOL_TOTAL_USED_PROP.to_string(),
                pool_used_to_prop,
                new_used,
                consts::POOL_ALLOC_SIZE_PROP.to_string(),
                pool_alloc_to_prop,
                new_alloc,
                consts::POOL_NO_ALLOCABLE_SPACE_PROP.to_string(),
                |x| x,
                new_no_space
            },
            consts::POOL_INTERFACE_NAME_3_2 => {
                consts::POOL_TOTAL_USED_PROP.to_string(),
                pool_used_to_prop,
                new_used,
                consts::POOL_ALLOC_SIZE_PROP.to_string(),
                pool_alloc_to_prop,
                new_alloc,
                consts::POOL_NO_ALLOCABLE_SPACE_PROP.to_string(),
                |x| x,
                new_no_space
            },
            consts::POOL_INTERFACE_NAME_3_3 => {
                consts::POOL_TOTAL_USED_PROP.to_string(),
                pool_used_to_prop,
                new_used,
                consts::POOL_ALLOC_SIZE_PROP.to_string(),
                pool_alloc_to_prop,
                new_alloc,
                consts::POOL_NO_ALLOCABLE_SPACE_PROP.to_string(),
                |x| x,
                new_no_space
            }
        );
    }

    /// Send a signal indicating that the pool total size has changed.
    fn handle_pool_size_change(&self, path: Path<'static>, new_size: Bytes) {
        if let Err(e) = self.property_changed_invalidated_signal(
            &path,
            prop_hashmap! {
                consts::POOL_INTERFACE_NAME_3_0 => {
                    Vec::new(),
                    consts::POOL_TOTAL_SIZE_PROP.to_string() =>
                    box_variant!(pool_size_to_prop(new_size))
                },
                consts::POOL_INTERFACE_NAME_3_1 => {
                    Vec::new(),
                    consts::POOL_TOTAL_SIZE_PROP.to_string() =>
                    box_variant!(pool_size_to_prop(new_size))
                },
                consts::POOL_INTERFACE_NAME_3_2 => {
                    Vec::new(),
                    consts::POOL_TOTAL_SIZE_PROP.to_string() =>
                    box_variant!(pool_size_to_prop(new_size))
                },
                consts::POOL_INTERFACE_NAME_3_3 => {
                    Vec::new(),
                    consts::POOL_TOTAL_SIZE_PROP.to_string() =>
                    box_variant!(pool_size_to_prop(new_size))
                }
            },
        ) {
            warn!(
                "Failed to send a signal over D-Bus indicating pool size change: {}",
                e
            );
        }
    }

    /// Send a signal indicating that the pool filesystem limit has changed.
    fn handle_pool_fs_limit_change(&self, path: Path<'static>, new_fs_limit: u64) {
        if let Err(e) = self.property_changed_invalidated_signal(
            &path,
            prop_hashmap!(
                consts::POOL_INTERFACE_NAME_3_1 => {
                    Vec::new(),
                    consts::POOL_FS_LIMIT_PROP.to_string() =>
                    box_variant!(new_fs_limit)
                },
                consts::POOL_INTERFACE_NAME_3_2 => {
                    Vec::new(),
                    consts::POOL_FS_LIMIT_PROP.to_string() =>
                    box_variant!(new_fs_limit)
                },
                consts::POOL_INTERFACE_NAME_3_3 => {
                    Vec::new(),
                    consts::POOL_FS_LIMIT_PROP.to_string() =>
                    box_variant!(new_fs_limit)
                }
            ),
        ) {
            warn!(
                "Failed to send a signal over D-Bus indicating pool size change: {}",
                e
            );
        }
    }

    /// Send a signal indicating that the blockdev user info has changed.
    fn handle_blockdev_user_info_change(&self, path: Path<'static>, new_user_info: Option<String>) {
        let user_info_prop = blockdev_user_info_to_prop(new_user_info);
        if let Err(e) = self.property_changed_invalidated_signal(
            &path,
            prop_hashmap!(
                consts::POOL_INTERFACE_NAME_3_1 => {
                    Vec::new(),
                    consts::BLOCKDEV_USER_INFO_PROP.to_string() =>
                    box_variant!(user_info_prop.clone())
                },
                consts::POOL_INTERFACE_NAME_3_2 => {
                    Vec::new(),
                    consts::BLOCKDEV_USER_INFO_PROP.to_string() =>
                    box_variant!(user_info_prop.clone())
                },
                consts::POOL_INTERFACE_NAME_3_3 => {
                    Vec::new(),
                    consts::BLOCKDEV_USER_INFO_PROP.to_string() =>
                    box_variant!(user_info_prop)
                }
            ),
        ) {
            warn!(
                "Failed to send a signal over D-Bus indicating blockdev user info change: {}",
                e
            );
        }
    }

    /// Send a signal indicating that the pool overprovisioning mode has changed.
    fn handle_pool_overprov_mode_change(&self, path: Path<'static>, new_mode: bool) {
        if let Err(e) = self.property_changed_invalidated_signal(
            &path,
            prop_hashmap!(
                consts::POOL_INTERFACE_NAME_3_1 => {
                    Vec::new(),
                    consts::POOL_OVERPROV_PROP.to_string() =>
                    box_variant!(new_mode)
                },
                consts::POOL_INTERFACE_NAME_3_2 => {
                    Vec::new(),
                    consts::POOL_OVERPROV_PROP.to_string() =>
                    box_variant!(new_mode)
                },
                consts::POOL_INTERFACE_NAME_3_3 => {
                    Vec::new(),
                    consts::POOL_OVERPROV_PROP.to_string() =>
                    box_variant!(new_mode)
                }
            ),
        ) {
            warn!(
                "Failed to send a signal over D-Bus indicating pool overprovisioning mode change: {}",
                e
            );
        }
    }

    /// Send a signal indicating that the pool total allocated size has changed.
    fn handle_pool_foreground_change(
        &self,
        path: Path<'static>,
        new_used: SignalChange<Option<Bytes>>,
        new_alloc: SignalChange<Bytes>,
        new_size: SignalChange<Bytes>,
        new_no_space: SignalChange<bool>,
    ) {
        handle_signal_change!(
            self,
            &path,
            "pool",
            consts::POOL_INTERFACE_NAME_3_0 => {
                consts::POOL_TOTAL_USED_PROP.to_string(),
                pool_used_to_prop,
                new_used,
                consts::POOL_ALLOC_SIZE_PROP.to_string(),
                pool_alloc_to_prop,
                new_alloc,
                consts::POOL_TOTAL_SIZE_PROP.to_string(),
                pool_size_to_prop,
                new_size
            },
            consts::POOL_INTERFACE_NAME_3_1 => {
                consts::POOL_TOTAL_USED_PROP.to_string(),
                pool_used_to_prop,
                new_used,
                consts::POOL_ALLOC_SIZE_PROP.to_string(),
                pool_alloc_to_prop,
                new_alloc,
                consts::POOL_TOTAL_SIZE_PROP.to_string(),
                pool_size_to_prop,
                new_size,
                consts::POOL_NO_ALLOCABLE_SPACE_PROP.to_string(),
                |x| x,
                new_no_space
            },
            consts::POOL_INTERFACE_NAME_3_2 => {
                consts::POOL_TOTAL_USED_PROP.to_string(),
                pool_used_to_prop,
                new_used,
                consts::POOL_ALLOC_SIZE_PROP.to_string(),
                pool_alloc_to_prop,
                new_alloc,
                consts::POOL_TOTAL_SIZE_PROP.to_string(),
                pool_size_to_prop,
                new_size,
                consts::POOL_NO_ALLOCABLE_SPACE_PROP.to_string(),
                |x| x,
                new_no_space
            },
            consts::POOL_INTERFACE_NAME_3_3 => {
                consts::POOL_TOTAL_USED_PROP.to_string(),
                pool_used_to_prop,
                new_used,
                consts::POOL_ALLOC_SIZE_PROP.to_string(),
                pool_alloc_to_prop,
                new_alloc,
                consts::POOL_TOTAL_SIZE_PROP.to_string(),
                pool_size_to_prop,
                new_size,
                consts::POOL_NO_ALLOCABLE_SPACE_PROP.to_string(),
                |x| x,
                new_no_space
            }
        );
    }

    fn handle_udev_background_change(
        &self,
        read_lock: TreeReadLock<E>,
        uuid: DevUuid,
        new_size: SignalChange<Option<Sectors>>,
    ) {
        handle_background_change!(
            self,
            read_lock,
            uuid,
            Dev,
            "blockdev",
            consts::BLOCKDEV_INTERFACE_NAME_3_3 => {
                consts::BLOCKDEV_NEW_SIZE_PROP.to_string(),
                blockdev_new_size_to_prop,
                new_size
            }
        )
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
            DbusAction::PoolCacheChange(item, has_cache) => {
                self.handle_pool_cache_change(item, has_cache);
                Ok(true)
            }
            DbusAction::PoolSizeChange(path, new_size) => {
                self.handle_pool_size_change(path, new_size);
                Ok(true)
            }
            DbusAction::PoolFsLimitChange(path, new_limit) => {
                self.handle_pool_fs_limit_change(path, new_limit);
                Ok(true)
            }
            DbusAction::PoolOverprovModeChange(path, new_mode) => {
                self.handle_pool_overprov_mode_change(path, new_mode);
                Ok(true)
            }
            DbusAction::LockedPoolsChange(pools) => {
                self.handle_locked_pools_change(pools);
                Ok(true)
            }
            DbusAction::StoppedPoolsChange(pools) => {
                self.handle_stopped_pools_change(pools);
                Ok(true)
            }
            DbusAction::BlockdevUserInfoChange(path, new_user_info) => {
                self.handle_blockdev_user_info_change(path, new_user_info);
                Ok(true)
            }
            DbusAction::PoolForegroundChange(item, new_used, new_alloc, new_size, new_no_space) => {
                self.handle_pool_foreground_change(
                    item,
                    new_used,
                    new_alloc,
                    new_size,
                    new_no_space,
                );
                Ok(true)
            }
            DbusAction::FsBackgroundChange(uuid, new_used, new_size) => {
                background_arm! {
                    self,
                    uuid,
                    handle_fs_background_change,
                    new_used,
                    new_size
                }
            }
            DbusAction::PoolBackgroundChange(uuid, new_used, new_alloc, new_no_space) => {
                background_arm! {
                    self,
                    uuid,
                    handle_pool_background_change,
                    new_used,
                    new_alloc,
                    new_no_space
                }
            }
            DbusAction::UdevBackgroundChange(uuid, new_size) => {
                background_arm! {
                    self,
                    uuid,
                    handle_udev_background_change,
                    new_size
                }
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
        props: PropertySignal,
    ) -> Result<(), dbus::Error> {
        props.into_iter().try_for_each(
            |(interface_name, (changed_properties, invalidated_properties))| {
                let prop_changed = PropertiesPropertiesChanged {
                    changed_properties,
                    invalidated_properties,
                    interface_name,
                };
                self.connection
                    .send(prop_changed.to_emit_message(object))
                    .map(|_| ())
                    .map_err(|_| {
                        dbus::Error::new_failed("Failed to send the requested signal on the D-Bus.")
                    })
            },
        )
    }
}

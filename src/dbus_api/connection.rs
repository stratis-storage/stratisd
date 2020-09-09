// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{cell::RefCell, rc::Rc};

use dbus::{
    ffidisp::{
        stdintf::org_freedesktop_dbus::{
            ObjectManagerInterfacesAdded, ObjectManagerInterfacesRemoved,
        },
        BusType, Connection, ConnectionItem, NameFlag, WatchEvent,
    },
    message::SignalArgs,
    strings::Path,
    tree::{MTFn, Tree},
};

use crate::{
    dbus_api::{
        api::get_base_tree,
        blockdev::create_dbus_blockdev,
        consts,
        filesystem::create_dbus_filesystem,
        pool::create_dbus_pool,
        types::{DbusContext, DeferredAction, InterfacesAdded, InterfacesRemoved, TData},
    },
    engine::{Engine, Name, Pool, PoolUuid},
};

/// Returned data from when you connect a stratis engine to dbus.
pub struct DbusConnectionData {
    pub connection: Connection,
    pub tree: Tree<MTFn<TData>, TData>,
    pub path: dbus::Path<'static>,
    pub context: DbusContext,
}

impl DbusConnectionData {
    /// Connect a stratis engine to dbus.
    pub fn connect(engine: Rc<RefCell<dyn Engine>>) -> Result<DbusConnectionData, dbus::Error> {
        let c = Connection::get_private(BusType::System)?;
        let (tree, object_path) = get_base_tree(DbusContext::new(engine));
        let dbus_context = tree.get_data().clone();
        tree.set_registered(&c, true)?;
        c.register_name(
            consts::STRATIS_BASE_SERVICE,
            NameFlag::ReplaceExisting as u32,
        )?;
        Ok(DbusConnectionData {
            connection: c,
            tree,
            path: object_path,
            context: dbus_context,
        })
    }

    /// Given the UUID of a pool, register all the pertinent information with dbus.
    pub fn register_pool(&mut self, pool_name: &Name, pool_uuid: PoolUuid, pool: &dyn Pool) {
        let pool_path =
            create_dbus_pool(&self.context, self.path.clone(), pool_name, pool_uuid, pool);
        for (fs_name, fs_uuid, fs) in pool.filesystems() {
            create_dbus_filesystem(
                &self.context,
                pool_path.clone(),
                pool_name,
                &fs_name,
                fs_uuid,
                fs,
            );
        }
        for (uuid, tier, bd) in pool.blockdevs() {
            create_dbus_blockdev(&self.context, pool_path.clone(), uuid, tier, bd);
        }

        self.process_deferred_actions()
    }

    /// Update the dbus tree with deferred adds and removes.
    fn process_deferred_actions(&mut self) {
        let mut actions = self.context.actions.borrow_mut();
        for action in actions.drain() {
            match action {
                DeferredAction::Add(path, interfaces) => {
                    let path_name = path.get_name().clone();
                    self.connection
                        .register_object_path(&path_name)
                        .expect("Must succeed since object paths are unique");
                    self.tree.insert(path);
                    if let Err(e) = self.added_object_signal(path_name, interfaces) {
                        warn!("Failed to send a signal on D-Bus object addition: {}", e);
                    }
                }
                DeferredAction::Remove(path, interfaces) => {
                    self.connection.unregister_object_path(&path);
                    self.tree.remove(&path);
                    if let Err(e) = self.removed_object_signal(path, interfaces) {
                        warn!("Failed to send a signal on D-Bus object removal: {}", e);
                    }
                }
            }
        }
    }

    /// Handle any client dbus requests
    pub fn handle(&mut self, fds: &[libc::pollfd]) {
        for pfd in fds.iter().filter(|pfd| pfd.revents != 0) {
            let items: Vec<ConnectionItem> = self
                .connection
                .watch_handle(pfd.fd, WatchEvent::from_revents(pfd.revents))
                .collect();

            for item in items {
                if let ConnectionItem::MethodCall(ref msg) = item {
                    if let Some(v) = self.tree.handle(msg) {
                        // Probably the wisest is to ignore any send errors here -
                        // maybe the remote has disconnected during our processing.
                        for m in v {
                            let _ = self.connection.send(m);
                        }
                    }

                    self.process_deferred_actions();
                }
            }
        }
    }

    // Send an InterfacesAdded signal on the D-Bus
    fn added_object_signal(
        &self,
        object: Path<'static>,
        interfaces: InterfacesAdded,
    ) -> Result<(), dbus::Error> {
        self.connection
            .send(
                ObjectManagerInterfacesAdded { object, interfaces }
                    .to_emit_message(&Path::from(consts::STRATIS_BASE_PATH)),
            )
            .map(|_| ())
            .map_err(|_| {
                dbus::Error::new_failed("Failed to send the requested signal on the D-Bus.")
            })
    }

    // Send an InterfacesRemoved signal on the D-Bus
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

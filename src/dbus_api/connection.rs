// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{cell::RefCell, collections::HashMap, rc::Rc, vec::Vec};

use dbus::{
    arg::{RefArg, Variant},
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
        types::{DbusContext, DeferredAction, TData},
    },
    engine::{Engine, Pool, PoolUuid},
};

/// Returned data from when you connect a stratis engine to dbus.
pub struct DbusConnectionData {
    pub connection: Rc<RefCell<Connection>>,
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
            connection: Rc::new(RefCell::new(c)),
            tree,
            path: object_path,
            context: dbus_context,
        })
    }

    /// Given the UUID of a pool, register all the pertinent information with dbus.
    pub fn register_pool(&mut self, pool_uuid: PoolUuid, pool: &mut dyn Pool) {
        let pool_path = create_dbus_pool(&self.context, self.path.clone(), pool_uuid, pool);
        for (_, fs_uuid, fs) in pool.filesystems_mut() {
            create_dbus_filesystem(&self.context, pool_path.clone(), fs_uuid, fs);
        }
        for (uuid, bd) in pool.blockdevs_mut() {
            create_dbus_blockdev(&self.context, pool_path.clone(), uuid, bd);
        }

        self.process_deferred_actions()
    }

    /// Update the dbus tree with deferred adds and removes.
    fn process_deferred_actions(&mut self) {
        let mut actions = self.context.actions.borrow_mut();
        for action in actions.drain() {
            match action {
                DeferredAction::Add(path) => {
                    self.connection
                        .borrow_mut()
                        .register_object_path(path.get_name())
                        .expect("Must succeed since object paths are unique");
                    self.tree.insert(path);
                }
                DeferredAction::Remove(path) => {
                    self.connection.borrow_mut().unregister_object_path(&path);
                    self.tree.remove(&path);
                }
            }
        }
    }

    /// Handle any client dbus requests
    pub fn handle(&mut self, fds: &[libc::pollfd]) {
        for pfd in fds.iter().filter(|pfd| pfd.revents != 0) {
            let items: Vec<ConnectionItem> = self
                .connection
                .borrow()
                .watch_handle(pfd.fd, WatchEvent::from_revents(pfd.revents))
                .collect();

            for item in items {
                if let ConnectionItem::MethodCall(ref msg) = item {
                    if let Some(v) = self.tree.handle(msg) {
                        // Probably the wisest is to ignore any send errors here -
                        // maybe the remote has disconnected during our processing.
                        for m in v {
                            let _ = self.connection.borrow_mut().send(m);
                        }
                    }

                    self.process_deferred_actions();
                }
            }
        }
    }

    /// Send an InterfacesAdded signal on the D-Bus
    pub fn added_object_signal(
        &mut self,
        object: Path<'static>,
        interfaces: HashMap<String, HashMap<String, Variant<Box<dyn RefArg>>>>,
    ) -> Result<(), dbus::Error> {
        self.connection
            .borrow()
            .send(
                ObjectManagerInterfacesAdded { object, interfaces }
                    .to_emit_message(&Path::from(consts::STRATIS_BASE_PATH)),
            )
            .map(|_| ())
            .map_err(|_| {
                dbus::Error::new_failed("Failed to send the requested signal on the D-Bus.")
            })
    }

    /// Send an InterfacesRemoved signal on the D-Bus
    pub fn removed_object_signal(
        &mut self,
        object: Path<'static>,
        interfaces: Vec<String>,
    ) -> Result<(), dbus::Error> {
        self.connection
            .borrow()
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

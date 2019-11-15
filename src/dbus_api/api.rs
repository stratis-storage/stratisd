// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    vec::Vec,
};

use dbus::{
    self,
    arg::{Array, IterAppend},
    tree::{
        Access, EmitsChangedSignal, Factory, MTFn, Method, MethodErr, MethodInfo, MethodResult,
        PropInfo, Property, Tree,
    },
    BusType, Connection, ConnectionItem, Message, NameFlag,
};
use libc;

use crate::{
    dbus_api::{
        blockdev::create_dbus_blockdev,
        consts,
        filesystem::create_dbus_filesystem,
        pool::create_dbus_pool,
        types::{DbusContext, DeferredAction, TData},
        util::{
            engine_to_dbus_err_tuple, get_next_arg, msg_code_ok, msg_string_ok, tuple_to_option,
        },
    },
    engine::{CreateAction, DeleteAction, Engine, Pool, PoolUuid},
    stratis::VERSION,
};

fn create_pool_shared(m: &MethodInfo<MTFn<TData>, TData>, has_keyfile: bool) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let name: &str = get_next_arg(&mut iter, 0)?;
    let redundancy_tuple: (bool, u16) = get_next_arg(&mut iter, 1)?;
    let devs: Array<&str, _> = get_next_arg(&mut iter, 2)?;
    let keyfile_tuple: Option<(bool, &str)> = if has_keyfile {
        Some(get_next_arg(&mut iter, 3)?)
    } else {
        None
    };

    let object_path = m.path.get_name();
    let dbus_context = m.tree.get_data();
    let mut engine = dbus_context.engine.borrow_mut();
    let result = engine.create_pool(
        name,
        &devs.map(|x| Path::new(x)).collect::<Vec<&Path>>(),
        tuple_to_option(redundancy_tuple),
        keyfile_tuple.and_then(|kt| tuple_to_option(kt).map(PathBuf::from)),
    );

    let return_message = message.method_return();

    let default_return: (bool, (dbus::Path<'static>, Vec<dbus::Path<'static>>)) =
        (false, (dbus::Path::default(), Vec::new()));

    let msg = match result {
        Ok(pool_uuid_action) => {
            let results = match pool_uuid_action {
                CreateAction::Created(uuid) => {
                    let (_, pool) = get_mut_pool!(engine; uuid; default_return; return_message);

                    let pool_object_path: dbus::Path =
                        create_dbus_pool(dbus_context, object_path.clone(), uuid, pool);

                    let bd_paths = pool
                        .blockdevs_mut()
                        .into_iter()
                        .map(|(uuid, bd)| {
                            create_dbus_blockdev(dbus_context, pool_object_path.clone(), uuid, bd)
                        })
                        .collect::<Vec<_>>();
                    (true, (pool_object_path, bd_paths))
                }
                CreateAction::Identity => default_return,
            };
            return_message.append3(results, msg_code_ok(), msg_string_ok())
        }
        Err(x) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&x);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn create_pool_2_0(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    create_pool_shared(m, false)
}

fn create_pool_2_1(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    create_pool_shared(m, true)
}

fn destroy_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let pool_path: dbus::Path<'static> = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();

    let default_return = (false, uuid_to_string!(PoolUuid::nil()));
    let return_message = message.method_return();

    let pool_uuid = match m
        .tree
        .get(&pool_path)
        .and_then(|op| op.get_data().as_ref())
        .map(|d| d.uuid)
    {
        Some(uuid) => uuid,
        None => {
            return Ok(vec![return_message.append3(
                default_return,
                msg_code_ok(),
                msg_string_ok(),
            )]);
        }
    };

    let msg = match dbus_context.engine.borrow_mut().destroy_pool(pool_uuid) {
        Ok(DeleteAction::Deleted(uuid)) => {
            dbus_context
                .actions
                .borrow_mut()
                .push_remove(&pool_path, m.tree);
            return_message.append3(
                (true, uuid_to_string!(uuid)),
                msg_code_ok(),
                msg_string_ok(),
            )
        }
        Ok(DeleteAction::Identity) => {
            return_message.append3(default_return, msg_code_ok(), msg_string_ok())
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn get_version(i: &mut IterAppend, _p: &PropInfo<MTFn<TData>, TData>) -> Result<(), MethodErr> {
    i.append(VERSION);
    Ok(())
}

fn configure_simulator(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message = m.msg;
    let mut iter = message.iter_init();

    let denominator: u32 = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let result = dbus_context
        .engine
        .borrow_mut()
        .configure_simulator(denominator);

    let return_message = message.method_return();

    let msg = match result {
        Ok(_) => return_message.append2(msg_code_ok(), msg_string_ok()),
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append2(rc, rs)
        }
    };
    Ok(vec![msg])
}

fn get_base_tree<'a>(dbus_context: DbusContext) -> (Tree<MTFn<TData>, TData>, dbus::Path<'a>) {
    let f = Factory::new_fn();

    let base_tree = f.tree(dbus_context);

    let create_pool_method = f
        .method("CreatePool", (), create_pool_2_0)
        .in_arg(("name", "s"))
        .in_arg(("redundancy", "(bq)"))
        .in_arg(("devices", "as"))
        // In order from left to right:
        // b: true if a pool was created and object paths were returned
        // o: Object path for Pool
        // a(o): Array of object paths for block devices
        //
        // Rust representation: (bool, (dbus::Path, Vec<dbus::Path>))
        .out_arg(("result", "(b(oao))"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let create_pool_method_2_1 = f
        .method("CreatePool", (), create_pool_2_1)
        .in_arg(("name", "s"))
        .in_arg(("redundancy", "(bq)"))
        .in_arg(("devices", "as"))
        // Optional keyfile
        // b: true if the pool should be encrypted
        // s: Path to keyfile
        //
        // Rust representation: (bool, String)
        .in_arg(("keyfile", "(bs)"))
        // In order from left to right:
        // b: true if a pool was created and object paths were returned
        // o: Object path for Pool
        // a(o): Array of object paths for block devices
        //
        // Rust representation: (bool, (dbus::Path, Vec<dbus::Path>))
        .out_arg(("result", "(b(oao))"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    fn destroy_pool_method_object(f: &Factory<MTFn<TData>, TData>) -> Method<MTFn<TData>, TData> {
        f.method("DestroyPool", (), destroy_pool)
            .in_arg(("pool", "o"))
            // In order from left to right:
            // b: true if a valid UUID is returned - otherwise no action was performed
            // s: String representation of pool UUID that was destroyed
            //
            // Rust representation: (bool, String)
            .out_arg(("result", "(bs)"))
            .out_arg(("return_code", "q"))
            .out_arg(("return_string", "s"))
    }

    fn configure_simulator_method_object(
        f: &Factory<MTFn<TData>, TData>,
    ) -> Method<MTFn<TData>, TData> {
        f.method("ConfigureSimulator", (), configure_simulator)
            .in_arg(("denominator", "u"))
            .out_arg(("return_code", "q"))
            .out_arg(("return_string", "s"))
    }

    fn version_property_object(f: &Factory<MTFn<TData>, TData>) -> Property<MTFn<TData>, TData> {
        f.property::<&str, _>("Version", ())
            .access(Access::Read)
            .emits_changed(EmitsChangedSignal::Const)
            .on_get(get_version)
    }

    let obj_path = f
        .object_path(consts::STRATIS_BASE_PATH, None)
        .introspectable()
        .object_manager()
        .add(
            f.interface(consts::MANAGER_INTERFACE_NAME, ())
                .add_m(create_pool_method)
                .add_m(destroy_pool_method_object(&f))
                .add_m(configure_simulator_method_object(&f))
                .add_p(version_property_object(&f)),
        )
        .add(
            f.interface(consts::MANAGER_INTERFACE_NAME_2_1, ())
                .add_m(create_pool_method_2_1)
                .add_m(destroy_pool_method_object(&f))
                .add_m(configure_simulator_method_object(&f))
                .add_p(version_property_object(&f)),
        );

    let path = obj_path.get_name().to_owned();
    (base_tree.add(obj_path), path)
}

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
                .watch_handle(pfd.fd, dbus::WatchEvent::from_revents(pfd.revents))
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
}

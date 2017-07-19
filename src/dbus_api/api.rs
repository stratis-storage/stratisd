// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::borrow::Cow;
use std::fmt::Display;
use std::path::Path;
use std::vec::Vec;
use std::rc::Rc;
use std::cell::RefCell;

use dbus;
use dbus::Connection;
use dbus::BusType;
use dbus::Message;
use dbus::MessageItem;
use dbus::NameFlag;
use dbus::arg::Array;
use dbus::arg::IterAppend;
use dbus::tree::Access;
use dbus::tree::EmitsChangedSignal;
use dbus::tree::Factory;
use dbus::tree::MethodErr;
use dbus::tree::MTFn;
use dbus::tree::MethodResult;
use dbus::tree::MethodInfo;
use dbus::tree::PropInfo;
use dbus::tree::Tree;
use dbus::ConnectionItem;

use engine::{Engine, Redundancy};
use stratis::VERSION;

use super::pool::create_dbus_pool;
use super::types::{DeferredAction, DbusContext, DbusErrorEnum, TData};
use super::util::STRATIS_BASE_PATH;
use super::util::STRATIS_BASE_SERVICE;
use super::util::code_to_message_items;
use super::util::default_object_path;
use super::util::engine_to_dbus_err;
use super::util::get_next_arg;
use super::util::ok_message_items;
use super::util::tuple_to_option;

fn create_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let name: &str = try!(get_next_arg(&mut iter, 0));
    let redundancy: (bool, u16) = try!(get_next_arg(&mut iter, 1));
    let force: bool = try!(get_next_arg(&mut iter, 2));
    let devs: Array<&str, _> = try!(get_next_arg(&mut iter, 3));

    let blockdevs = devs.map(|x| Path::new(x)).collect::<Vec<&Path>>();

    let object_path = m.path.get_name();
    let dbus_context = m.tree.get_data();
    let result = dbus_context
        .engine
        .borrow_mut()
        .create_pool(name, &blockdevs, tuple_to_option(redundancy), force);

    let return_message = message.method_return();

    let msg = match result {
        Ok((uuid, devnodes)) => {
            let pool_object_path: dbus::Path =
                create_dbus_pool(dbus_context, object_path.clone(), uuid);
            let paths = devnodes
                .iter()
                .map(|d| {
                         d.to_str()
                             .expect("'d' originated in the 'devs' D-Bus argument.")
                             .into()
                     });
            let paths = paths.map(MessageItem::Str).collect();
            let return_path = MessageItem::ObjectPath(pool_object_path);
            let return_list = MessageItem::Array(paths, "s".into());
            let return_value = MessageItem::Struct(vec![return_path, return_list]);
            let (rc, rs) = ok_message_items();
            return_message.append3(return_value, rc, rs)
        }
        Err(x) => {
            let return_path = MessageItem::ObjectPath(default_object_path());
            let return_list = MessageItem::Array(vec![], "s".into());
            let return_value = MessageItem::Struct(vec![return_path, return_list]);
            let (rc, rs) = engine_to_dbus_err(&x);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(return_value, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn destroy_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {

    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let object_path: dbus::Path<'static> = try!(get_next_arg(&mut iter, 0));

    let dbus_context = m.tree.get_data();

    let default_return = MessageItem::Bool(false);
    let return_message = message.method_return();

    let pool_uuid = match m.tree.get(&object_path) {
        Some(pool_path) => get_data!(pool_path; default_return; return_message).uuid,
        None => {
            let (rc, rs) = ok_message_items();
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };

    let msg = match dbus_context
              .engine
              .borrow_mut()
              .destroy_pool(&pool_uuid) {
        Ok(action) => {
            dbus_context
                .actions
                .borrow_mut()
                .push_remove(object_path);
            let (rc, rs) = ok_message_items();
            return_message.append3(MessageItem::Bool(action), rc, rs)
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err(&err);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn get_list_items<T, I>(i: &mut IterAppend, iter: I) -> Result<(), MethodErr>
    where T: Display + Into<u16>,
          I: Iterator<Item = T>
{
    let msg_vec = iter.map(|item| {
                               MessageItem::Struct(vec![MessageItem::Str(format!("{}", item)),
                                                        MessageItem::UInt16(item.into())])
                           })
        .collect::<Vec<MessageItem>>();
    i.append(MessageItem::Array(msg_vec, Cow::Borrowed("(sq)")));
    Ok(())
}

fn get_error_values(i: &mut IterAppend,
                    _p: &PropInfo<MTFn<TData>, TData>)
                    -> Result<(), MethodErr> {
    get_list_items(i, DbusErrorEnum::iter_variants())
}


fn get_redundancy_values(i: &mut IterAppend,
                         _p: &PropInfo<MTFn<TData>, TData>)
                         -> Result<(), MethodErr> {
    get_list_items(i, Redundancy::iter_variants())
}

fn get_version(i: &mut IterAppend, _p: &PropInfo<MTFn<TData>, TData>) -> Result<(), MethodErr> {
    i.append(VERSION);
    Ok(())
}

fn configure_simulator(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message = m.msg;
    let mut iter = message.iter_init();

    let denominator: u32 = try!(get_next_arg(&mut iter, 0));

    let dbus_context = m.tree.get_data();
    let result = dbus_context
        .engine
        .borrow_mut()
        .configure_simulator(denominator);

    let return_message = message.method_return();

    let msg = match result {
        Ok(_) => {
            let (rc, rs) = ok_message_items();
            return_message.append2(rc, rs)
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err(&err);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append2(rc, rs)
        }
    };
    Ok(vec![msg])
}

fn get_base_tree(dbus_context: DbusContext) -> Tree<MTFn<TData>, TData> {

    let f = Factory::new_fn();

    let base_tree = f.tree(dbus_context);

    let create_pool_method = f.method("CreatePool", (), create_pool)
        .in_arg(("name", "s"))
        .in_arg(("redundancy", "(bq)"))
        .in_arg(("force", "b"))
        .in_arg(("devices", "as"))
        .out_arg(("result", "(oas)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let destroy_pool_method = f.method("DestroyPool", (), destroy_pool)
        .in_arg(("pool", "o"))
        .out_arg(("action", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let configure_simulator_method = f.method("ConfigureSimulator", (), configure_simulator)
        .in_arg(("denominator", "u"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let redundancy_values_property =
        f.property::<Array<(&str, u16), &Iterator<Item = (&str, u16)>>, _>("RedundancyValues", ())
            .access(Access::Read)
            .emits_changed(EmitsChangedSignal::Const)
            .on_get(get_redundancy_values);

    let error_values_property =
        f.property::<Array<(&str, u16), &Iterator<Item = (&str, u16)>>, _>("ErrorValues", ())
            .access(Access::Read)
            .emits_changed(EmitsChangedSignal::Const)
            .on_get(get_error_values);

    let version_property = f.property::<&str, _>("Version", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_version);

    let interface_name = format!("{}.{}", STRATIS_BASE_SERVICE, "Manager");

    let obj_path = f.object_path(STRATIS_BASE_PATH, None)
        .introspectable()
        .object_manager()
        .add(f.interface(interface_name, ())
                 .add_m(create_pool_method)
                 .add_m(destroy_pool_method)
                 .add_m(configure_simulator_method)
                 .add_p(error_values_property)
                 .add_p(redundancy_values_property)
                 .add_p(version_property));

    base_tree.add(obj_path)
}

#[allow(type_complexity)]
pub fn connect(engine: Rc<RefCell<Engine>>)
               -> Result<(Connection, Tree<MTFn<TData>, TData>, DbusContext), dbus::Error> {
    let c = try!(Connection::get_private(BusType::System));

    let tree = get_base_tree(DbusContext::new(engine));
    let dbus_context = tree.get_data().clone();
    try!(tree.set_registered(&c, true));

    try!(c.register_name(STRATIS_BASE_SERVICE, NameFlag::ReplaceExisting as u32));

    Ok((c, tree, dbus_context))
}

pub fn handle(c: &Connection,
              item: &ConnectionItem,
              tree: &mut Tree<MTFn<TData>, TData>,
              dbus_context: &DbusContext)
              -> Result<(), dbus::Error> {
    if let ConnectionItem::MethodCall(ref msg) = *item {
        if let Some(v) = tree.handle(msg) {
            // Probably the wisest is to ignore any send errors here -
            // maybe the remote has disconnected during our processing.
            for m in v {
                let _ = c.send(m);
            }
        }

        let mut b_actions = dbus_context.actions.borrow_mut();
        for action in b_actions.drain() {
            match action {
                DeferredAction::Add(path) => {
                    try!(c.register_object_path(path.get_name()));
                    tree.insert(path);
                }
                DeferredAction::Remove(path) => {
                    c.unregister_object_path(&path);
                    tree.remove(&path);
                }
            }
        }
    }

    Ok(())
}

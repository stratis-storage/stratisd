// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;
use std::path::Path;
use std::vec::Vec;

use dbus;
use dbus::Message;
use dbus::MessageItem;
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

use uuid::Uuid;

use engine::{Pool, RenameAction};

use super::filesystem::create_dbus_filesystem;
use super::types::{DbusContext, DbusErrorEnum, OPContext, TData};

use super::util::STRATIS_BASE_PATH;
use super::util::STRATIS_BASE_SERVICE;
use super::util::code_to_message_items;
use super::util::engine_to_dbus_err;
use super::util::get_next_arg;
use super::util::get_uuid;
use super::util::ok_message_items;


fn create_filesystems(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let filesystems: Array<&str, _> = try!(get_next_arg(&mut iter, 0));
    let dbus_context = m.tree.get_data();

    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "(os)";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_path = m.tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = &get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let pool = get_pool!(engine; pool_uuid; default_return; return_message);

    let result = pool.create_filesystems(&filesystems.collect::<Vec<&str>>());

    let msg = match result {
        Ok(ref infos) => {
            let mut return_value = Vec::new();
            for &(name, uuid) in infos {
                let fs_object_path: dbus::Path =
                    create_dbus_filesystem(dbus_context, object_path.clone(), uuid);
                return_value.push((fs_object_path, name));
            }

            let return_value = return_value
                .iter()
                .map(|x| {
                         MessageItem::Struct(vec![MessageItem::ObjectPath(x.0.clone()),
                                                  MessageItem::Str((*x.1).into())])
                     })
                .collect();
            let return_value = MessageItem::Array(return_value, return_sig.into());
            let (rc, rs) = ok_message_items();
            return_message.append3(return_value, rc, rs)
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err(&err);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])

}

fn destroy_filesystems(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let filesystems: Array<dbus::Path<'static>, _> = try!(get_next_arg(&mut iter, 0));

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "s";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_path = m.tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = &get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let pool = get_pool!(engine; pool_uuid; default_return; return_message);

    let mut filesystem_map: HashMap<Uuid, dbus::Path<'static>> = HashMap::new();
    for op in filesystems {
        if let Some(filesystem_path) = m.tree.get(&op) {
            let filesystem_uuid = get_data!(filesystem_path; default_return; return_message).uuid;
            filesystem_map.insert(filesystem_uuid.clone(), op);
        }
    }

    let result = pool.destroy_filesystems(&filesystem_map.keys().collect::<Vec<&Uuid>>());
    let msg = match result {
        Ok(ref uuids) => {
            for uuid in uuids {
                let op = filesystem_map
                    .get(uuid)
                    .expect("'uuids' is a subset of filesystem_map.keys()")
                    .clone();
                dbus_context.actions.borrow_mut().push_remove(op);
            }

            let return_value = uuids
                .iter()
                .map(|n| MessageItem::Str(format!("{}", n.simple())))
                .collect();
            let (rc, rs) = ok_message_items();
            return_message.append3(MessageItem::Array(return_value, return_sig.into()), rc, rs)
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err(&err);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn add_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let force: bool = try!(get_next_arg(&mut iter, 0));
    let devs: Array<&str, _> = try!(get_next_arg(&mut iter, 1));

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "s";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_path = m.tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = &get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let pool = get_pool!(engine; pool_uuid; default_return; return_message);

    let blockdevs = devs.map(|x| Path::new(x)).collect::<Vec<&Path>>();

    let msg = match pool.add_blockdevs(&blockdevs, force) {
        Ok(devnodes) => {
            let paths = devnodes
                .iter()
                .map(|d| {
                         d.to_str()
                             .expect("'d' originated in the 'devs' D-Bus argument.")
                             .into()
                     });
            let paths = paths.map(MessageItem::Str).collect();
            let (rc, rs) = ok_message_items();
            return_message.append3(MessageItem::Array(paths, return_sig.into()), rc, rs)
        }
        Err(x) => {
            let (rc, rs) = engine_to_dbus_err(&x);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn rename_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let new_name: &str = try!(get_next_arg(&mut iter, 0));

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = MessageItem::Bool(false);

    let pool_path = m.tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = &get_data!(pool_path; default_return; return_message).uuid;

    let msg = match dbus_context
              .engine
              .borrow_mut()
              .rename_pool(&pool_uuid, new_name) {
        Ok(RenameAction::NoSource) => {
            let error_message = format!("engine doesn't know about pool {}", pool_uuid);
            let (rc, rs) = code_to_message_items(DbusErrorEnum::INTERNAL_ERROR, error_message);
            return_message.append3(default_return, rc, rs)
        }
        Ok(RenameAction::Identity) => {
            let (rc, rs) = ok_message_items();
            return_message.append3(MessageItem::Bool(false), rc, rs)
        }
        Ok(RenameAction::Renamed) => {
            let (rc, rs) = ok_message_items();
            return_message.append3(MessageItem::Bool(true), rc, rs)
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err(&err);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

/// Get a pool property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to a
/// Pool and obtains the property from the pool.
fn get_pool_property<F>(i: &mut IterAppend,
                        p: &PropInfo<MTFn<TData>, TData>,
                        getter: F)
                        -> Result<(), MethodErr>
    where F: Fn(&Pool) -> Result<MessageItem, MethodErr>
{
    let dbus_context = p.tree.get_data();
    let object_path = p.path.get_name();
    let pool_path = p.tree
        .get(object_path)
        .expect("implicit argument must be in tree");

    let pool_uuid = try!(pool_path
                        .get_data()
                        .as_ref()
                        .ok_or_else(|| {
                                        MethodErr::failed(&format!("no data for object path {}",
                                                                   object_path))
                                    }))
            .uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let pool = try!(engine
                        .get_pool(&pool_uuid)
                        .ok_or(MethodErr::failed(&format!("no pool corresponding to uuid {}",
                                                          &pool_uuid))));

    i.append(try!(getter(pool)));
    Ok(())
}

fn get_pool_name(i: &mut IterAppend, p: &PropInfo<MTFn<TData>, TData>) -> Result<(), MethodErr> {
    get_pool_property(i, p, |p| Ok(MessageItem::Str(p.name().to_owned())))
}

fn get_pool_total_physical_used(i: &mut IterAppend,
                                p: &PropInfo<MTFn<TData>, TData>)
                                -> Result<(), MethodErr> {
    fn get_used(pool: &Pool) -> Result<MessageItem, MethodErr> {
        let err_func = |_| {
            MethodErr::failed(&format!("no total physical size computed for pool with uuid {}",
                                       pool.uuid()))
        };

        try!(pool.total_physical_used()
                 .map(|u| Ok(MessageItem::Str(format!("{}", *u))))
                 .map_err(err_func))
    }

    get_pool_property(i, p, get_used)
}

fn get_pool_total_physical_size(i: &mut IterAppend,
                                p: &PropInfo<MTFn<TData>, TData>)
                                -> Result<(), MethodErr> {
    get_pool_property(i,
                      p,
                      |p| Ok(MessageItem::Str(format!("{}", *p.total_physical_size()))))
}

pub fn create_dbus_pool<'a>(dbus_context: &DbusContext,
                            parent: dbus::Path<'static>,
                            uuid: Uuid)
                            -> dbus::Path<'a> {

    let f = Factory::new_fn();

    let create_filesystems_method = f.method("CreateFilesystems", (), create_filesystems)
        .in_arg(("specs", "as"))
        .out_arg(("filesystems", "a(os)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let destroy_filesystems_method = f.method("DestroyFilesystems", (), destroy_filesystems)
        .in_arg(("filesystems", "ao"))
        .out_arg(("results", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let add_devs_method = f.method("AddDevs", (), add_devs)
        .in_arg(("force", "b"))
        .in_arg(("devices", "as"))
        .out_arg(("results", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let rename_method = f.method("SetName", (), rename_pool)
        .in_arg(("name", "s"))
        .out_arg(("action", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let name_property = f.property::<&str, _>("Name", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::False)
        .on_get(get_pool_name);

    let total_physical_size_property = f.property::<&str, _>("TotalPhysicalSize", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::False)
        .on_get(get_pool_total_physical_size);

    let total_physical_used_property = f.property::<&str, _>("TotalPhysicalUsed", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::False)
        .on_get(get_pool_total_physical_used);

    let uuid_property = f.property::<&str, _>("Uuid", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_uuid);

    let object_name = format!("{}/{}",
                              STRATIS_BASE_PATH,
                              dbus_context.get_next_id().to_string());

    let interface_name = format!("{}.{}", STRATIS_BASE_SERVICE, "pool");

    let object_path = f.object_path(object_name, Some(OPContext::new(parent, uuid)))
        .introspectable()
        .add(f.interface(interface_name, ())
                 .add_m(create_filesystems_method)
                 .add_m(destroy_filesystems_method)
                 .add_m(add_devs_method)
                 .add_m(rename_method)
                 .add_p(name_property)
                 .add_p(total_physical_size_property)
                 .add_p(total_physical_used_property)
                 .add_p(uuid_property));

    let path = object_path.get_name().to_owned();
    dbus_context.actions.borrow_mut().push_add(object_path);
    path
}

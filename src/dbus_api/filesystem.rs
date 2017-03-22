// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus;
use dbus::Message;
use dbus::MessageItem;
use dbus::arg::IterAppend;
use dbus::tree::Access;
use dbus::tree::EmitsChangedSignal;
use dbus::tree::Factory;
use dbus::tree::MTFn;
use dbus::tree::MethodErr;
use dbus::tree::MethodInfo;
use dbus::tree::MethodResult;
use dbus::tree::PropInfo;

use uuid::Uuid;

use engine::RenameAction;

use super::types::{DbusContext, DbusErrorEnum, OPContext, TData};

use super::util::STRATIS_BASE_PATH;
use super::util::STRATIS_BASE_SERVICE;
use super::util::code_to_message_items;
use super::util::engine_to_dbus_err;
use super::util::get_next_arg;
use super::util::ok_message_items;
use super::util::ref_ok_or;


pub fn create_dbus_filesystem<'a>(dbus_context: &DbusContext,
                                  parent: dbus::Path<'static>,
                                  uuid: Uuid)
                                  -> dbus::Path<'a> {
    let f = Factory::new_fn();

    let rename_method = f.method("SetName", (), rename_filesystem)
        .in_arg(("name", "s"))
        .out_arg(("action", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let name_property = f.property::<&str, _>("Name", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::False)
        .on_get(get_filesystem_name);

    let pool_property = f.property::<&dbus::Path, _>("Pool", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_filesystem_pool);

    let uuid_property = f.property::<&str, _>("Uuid", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_filesystem_uuid);

    let object_name = format!("{}/{}",
                              STRATIS_BASE_PATH,
                              dbus_context.get_next_id().to_string());

    let interface_name = format!("{}.{}", STRATIS_BASE_SERVICE, "filesystem");

    let object_path = f.object_path(object_name, Some(OPContext::new(parent, uuid)))
        .introspectable()
        .add(f.interface(interface_name, ())
            .add_m(rename_method)
            .add_p(name_property)
            .add_p(pool_property)
            .add_p(uuid_property));

    let path = object_path.get_name().to_owned();
    dbus_context.actions.borrow_mut().push_add(object_path);
    path
}

fn rename_filesystem(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let new_name: &str = try!(get_next_arg(&mut iter, 0));

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = MessageItem::Bool(false);

    let filesystem_path = m.tree.get(&object_path).expect("implicit argument must be in tree");
    let filesystem_data = get_data!(filesystem_path; default_return; return_message);

    let pool_path = get_parent!(m; filesystem_data; default_return; return_message);
    let pool_uuid = &get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let pool = get_pool!(engine; pool_uuid; default_return; return_message);

    let msg = match pool.rename_filesystem(&filesystem_data.uuid, &new_name) {
        Ok(RenameAction::NoSource) => {
            let error_message = format!("pool {} doesn't know about filesystem {}",
                                        pool_uuid,
                                        filesystem_data.uuid);
            let (rc, rs) = code_to_message_items(DbusErrorEnum::INTERNAL_ERROR, error_message);
            return_message.append3(default_return, rc, rs)
        }
        Ok(RenameAction::Identity) => {
            let (rc, rs) = ok_message_items();
            return_message.append3(default_return, rc, rs)
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


fn get_filesystem_uuid(i: &mut IterAppend,
                       p: &PropInfo<MTFn<TData>, TData>)
                       -> Result<(), MethodErr> {
    let object_path = p.path.get_name();
    let filesystem_path = p.tree.get(&object_path).expect("implicit argument must be in tree");
    let data = try!(ref_ok_or(filesystem_path.get_data(),
                              MethodErr::failed(&format!("no data for object path {}",
                                                         &object_path))));
    i.append(MessageItem::Str(format!("{}", data.uuid.simple())));
    Ok(())
}


fn get_filesystem_name(i: &mut IterAppend,
                       p: &PropInfo<MTFn<TData>, TData>)
                       -> Result<(), MethodErr> {
    let dbus_context = p.tree.get_data();
    let object_path = p.path.get_name();

    let filesystem_path = p.tree.get(&object_path).expect("tree must contain implicit argument");
    let filesystem_data =
        try!(ref_ok_or(filesystem_path.get_data(),
                       MethodErr::failed(&format!("no data for object path {}", &object_path))));

    let pool_path = try!(p.tree
        .get(&filesystem_data.parent)
        .ok_or(MethodErr::failed(&format!("no path for parent object path {}",
                                          &filesystem_data.parent))));
    let pool_uuid = try!(ref_ok_or(pool_path.get_data(),
                                   MethodErr::failed(&format!("no data for object path {}",
                                                              &object_path))))
        .uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let pool = try!(engine.get_pool(&pool_uuid)
        .ok_or(MethodErr::failed(&format!("no pool corresponding to uuid {}", &pool_uuid))));

    let filesystem_uuid = &filesystem_data.uuid;
    i.append(try!(pool.get_filesystem(filesystem_uuid)
        .map(|x| MessageItem::Str(x.name().to_owned()))
        .ok_or(MethodErr::failed(&format!("no name for filesystem with uuid {}",
                                          &filesystem_uuid)))));
    Ok(())
}

fn get_filesystem_pool(i: &mut IterAppend,
                       p: &PropInfo<MTFn<TData>, TData>)
                       -> Result<(), MethodErr> {
    let object_path = p.path.get_name();
    let filesystem_path = p.tree.get(&object_path).expect("implicit argument must be in tree");
    let data = try!(ref_ok_or(filesystem_path.get_data(),
                              MethodErr::failed(&format!("no data for object path {}",
                                                         &object_path))));
    i.append(MessageItem::ObjectPath(data.parent.clone()));
    Ok(())
}

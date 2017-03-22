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


pub fn create_dbus_blockdev<'a>(dbus_context: &DbusContext,
                                parent: dbus::Path<'static>,
                                uuid: Uuid)
                                -> dbus::Path<'a> {
    let f = Factory::new_fn();

    let pool_property = f.property::<&dbus::Path, _>("Pool", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_blockdev_pool);

    let uuid_property = f.property::<&str, _>("Uuid", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_blockdev_uuid);

    let object_name = format!("{}/{}",
                              STRATIS_BASE_PATH,
                              dbus_context.get_next_id().to_string());

    let interface_name = format!("{}.{}", STRATIS_BASE_SERVICE, "blockdev");

    let object_path = f.object_path(object_name, Some(OPContext::new(parent, uuid)))
        .introspectable()
        .add(f.interface(interface_name, ())
            .add_p(pool_property)
            .add_p(uuid_property));

    let path = object_path.get_name().to_owned();
    dbus_context.actions.borrow_mut().push_add(object_path);
    path
}


fn get_blockdev_uuid(i: &mut IterAppend,
                     p: &PropInfo<MTFn<TData>, TData>)
                     -> Result<(), MethodErr> {
    let object_path = p.path.get_name();
    let blockdev_path = p.tree.get(&object_path).expect("implicit argument must be in tree");
    let data = try!(ref_ok_or(blockdev_path.get_data(),
                              MethodErr::failed(&format!("no data for object path {}",
                                                         &object_path))));
    i.append(MessageItem::Str(format!("{}", data.uuid.simple())));
    Ok(())
}


fn get_blockdev_pool(i: &mut IterAppend,
                     p: &PropInfo<MTFn<TData>, TData>)
                     -> Result<(), MethodErr> {
    let object_path = p.path.get_name();
    let blockdev_path = p.tree.get(&object_path).expect("implicit argument must be in tree");
    let data = try!(ref_ok_or(blockdev_path.get_data(),
                              MethodErr::failed(&format!("no data for object path {}",
                                                         &object_path))));
    i.append(MessageItem::ObjectPath(data.parent.clone()));
    Ok(())
}

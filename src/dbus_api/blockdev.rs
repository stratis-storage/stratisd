// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus;
use dbus::Message;
use dbus::arg::IterAppend;
use dbus::tree::{Access, EmitsChangedSignal, Factory, MTFn, MethodErr, MethodInfo, MethodResult,
                 PropInfo};

use uuid::Uuid;

use super::super::engine::{BlockDev, BlockDevState};

use super::types::{DbusContext, DbusErrorEnum, OPContext, TData};

use super::util::{STRATIS_BASE_PATH, STRATIS_BASE_SERVICE, get_next_arg, get_parent, get_uuid,
                  msg_code_ok, msg_string_ok};


pub fn create_dbus_blockdev<'a>(dbus_context: &DbusContext,
                                parent: dbus::Path<'static>,
                                uuid: Uuid)
                                -> dbus::Path<'a> {
    let f = Factory::new_fn();

    let set_userid_method = f.method("SetUserInfo", (), set_user_info)
        .in_arg(("id", "s"))
        .out_arg(("changed", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let devnode_property = f.property::<&str, _>("Devnode", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_blockdev_devnode);

    let hardware_info_property = f.property::<&str, _>("HardwareInfo", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_blockdev_hardware_info);

    let user_info_property = f.property::<&str, _>("UserInfo", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::False)
        .on_get(get_blockdev_user_info);

    let initialization_time_property = f.property::<u64, _>("InitializationTime", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_blockdev_initialization_time);

    let total_physical_size_property = f.property::<&str, _>("TotalPhysicalSize", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::False)
        .on_get(get_blockdev_physical_size);

    let state_property = f.property::<u16, _>("State", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::False)
        .on_get(get_blockdev_state);

    let pool_property = f.property::<&dbus::Path, _>("Pool", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_parent);

    let uuid_property = f.property::<&str, _>("Uuid", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_uuid);

    let object_name = format!("{}/{}",
                              STRATIS_BASE_PATH,
                              dbus_context.get_next_id().to_string());

    let interface_name = format!("{}.{}", STRATIS_BASE_SERVICE, "blockdev");

    let object_path = f.object_path(object_name, Some(OPContext::new(parent, uuid)))
        .introspectable()
        .add(f.interface(interface_name, ())
                 .add_m(set_userid_method)
                 .add_p(devnode_property)
                 .add_p(hardware_info_property)
                 .add_p(initialization_time_property)
                 .add_p(total_physical_size_property)
                 .add_p(pool_property)
                 .add_p(state_property)
                 .add_p(user_info_property)
                 .add_p(uuid_property));

    let path = object_path.get_name().to_owned();
    dbus_context.actions.borrow_mut().push_add(object_path);
    path
}

fn set_user_info(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let new_id: Option<&str> = match get_next_arg(&mut iter, 0)? {
        "" => None,
        val => Some(val),
    };

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = false;

    let blockdev_path = m.tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let blockdev_data = get_data!(blockdev_path; default_return; return_message);

    let pool_path = get_parent!(m; blockdev_data; default_return; return_message);
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let (pool_name, pool) = get_mut_pool!(engine; pool_uuid; default_return; return_message);

    let id_changed = {
        let blockdev = pool.get_mut_blockdev(blockdev_data.uuid)
            .ok_or_else(|| {
                            MethodErr::failed(&format!("no blockdev with uuid {}",
                                                       blockdev_data.uuid))
                        })?;

        blockdev.set_user_info(new_id)
    };

    // FIXME: engine should decide to save state, not this function
    if id_changed {
        pool.save_state(&pool_name)
            .map_err(|err| {
                         MethodErr::failed(&format!("Could not save state for object path {}: {}",
                                                    object_path,
                                                    err))
                     })?;
    }

    let msg = return_message.append3(id_changed, msg_code_ok(), msg_string_ok());

    Ok(vec![msg])
}


/// Get a blockdev property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to a
/// blockdev and obtains the property from the blockdev.
fn get_blockdev_property<F, R>(i: &mut IterAppend,
                               p: &PropInfo<MTFn<TData>, TData>,
                               getter: F)
                               -> Result<(), MethodErr>
    where F: Fn(&BlockDev) -> Result<R, MethodErr>,
          R: dbus::arg::Append
{
    let dbus_context = p.tree.get_data();
    let object_path = p.path.get_name();

    let blockdev_path = p.tree
        .get(object_path)
        .expect("tree must contain implicit argument");

    let blockdev_data =
        blockdev_path
            .get_data()
            .as_ref()
            .ok_or_else(|| MethodErr::failed(&format!("no data for object path {}", object_path)))?;

    let pool_path = p.tree
        .get(&blockdev_data.parent)
        .ok_or_else(|| {
                        MethodErr::failed(&format!("no path for parent object path {}",
                                                   &blockdev_data.parent))
                    })?;

    let pool_uuid = pool_path
        .get_data()
        .as_ref()
        .ok_or_else(|| MethodErr::failed(&format!("no data for object path {}", object_path)))?
        .uuid;

    let engine = dbus_context.engine.borrow();
    let (_, pool) =
        engine
            .get_pool(pool_uuid)
            .ok_or_else(|| {
                            MethodErr::failed(&format!("no pool corresponding to uuid {}",
                                                       &pool_uuid))
                        })?;
    let blockdev =
        pool.get_blockdev(blockdev_data.uuid)
            .ok_or_else(|| {
                            MethodErr::failed(&format!("no blockdev with uuid {}",
                                                       blockdev_data.uuid))
                        })?;
    i.append(getter(blockdev)?);
    Ok(())
}

/// Get the devnode for an object path.
fn get_blockdev_devnode(i: &mut IterAppend,
                        p: &PropInfo<MTFn<TData>, TData>)
                        -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |p| Ok(format!("{}", p.devnode().display())))
}

fn get_blockdev_hardware_info(i: &mut IterAppend,
                              p: &PropInfo<MTFn<TData>, TData>)
                              -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |p| Ok(p.hardware_info().unwrap_or("").to_owned()))
}

fn get_blockdev_user_info(i: &mut IterAppend,
                          p: &PropInfo<MTFn<TData>, TData>)
                          -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |p| Ok(p.user_info().unwrap_or("").to_owned()))
}

fn get_blockdev_initialization_time(i: &mut IterAppend,
                                    p: &PropInfo<MTFn<TData>, TData>)
                                    -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |p| Ok(p.initialization_time().timestamp() as u64))
}

fn get_blockdev_physical_size(i: &mut IterAppend,
                              p: &PropInfo<MTFn<TData>, TData>)
                              -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |p| Ok(format!("{}", *p.total_size())))
}

fn get_blockdev_state(i: &mut IterAppend,
                      p: &PropInfo<MTFn<TData>, TData>)
                      -> Result<(), MethodErr> {
    fn get_state(blockdev: &BlockDev) -> Result<u16, MethodErr> {
        let state: u16 = match blockdev.state() {
            BlockDevState::Missing => 0,
            BlockDevState::Bad => 1,
            BlockDevState::Spare => 2,
            BlockDevState::NotInUse => 3,
            BlockDevState::InUse => 4,
        };
        Ok(state)
    }

    get_blockdev_property(i, p, get_state)
}

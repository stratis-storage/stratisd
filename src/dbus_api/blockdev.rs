// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use dbus::{
    self,
    arg::{Array, IterAppend, RefArg, Variant},
    tree::{
        Access, EmitsChangedSignal, Factory, MTFn, MethodErr, MethodInfo, MethodResult, PropInfo,
        Tree,
    },
    Message, Path,
};
use itertools::Itertools;
use uuid::Uuid;

use crate::{
    dbus_api::{
        consts,
        types::{DbusContext, DbusErrorEnum, OPContext, TData},
        util::{
            engine_to_dbus_err_tuple, get_next_arg, get_parent, get_uuid, make_object_path,
            msg_code_ok, msg_string_ok, result_to_tuple, tuple_to_option,
        },
    },
    engine::{BlockDev, BlockDevTier, DevUuid, MaybeDbusPath, RenameAction},
};

pub fn create_dbus_blockdev<'a>(
    dbus_context: &DbusContext,
    parent: dbus::Path<'static>,
    uuid: Uuid,
    blockdev: &mut dyn BlockDev,
) -> dbus::Path<'a> {
    let f = Factory::new_fn();

    let set_userid_method = f
        .method("SetUserInfo", (), set_user_info)
        .in_arg(("id", "(bs)"))
        // b: false if no change to the user info
        // s: UUID of the changed device
        //
        // Rust representation: (bool, String)
        .out_arg(("changed", "(bs)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let devnode_property = f
        .property::<&str, _>("Devnode", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_blockdev_devnode);

    let hardware_info_property = f
        .property::<(bool, &str), _>("HardwareInfo", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_blockdev_hardware_info);

    let user_info_property = f
        .property::<(bool, &str), _>("UserInfo", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::False)
        .on_get(get_blockdev_user_info);

    let initialization_time_property = f
        .property::<u64, _>("InitializationTime", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_blockdev_initialization_time);

    let pool_property = f
        .property::<&dbus::Path, _>("Pool", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_parent);

    let uuid_property = f
        .property::<&str, _>("Uuid", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_uuid);

    let tier_property = f
        .property::<u16, _>("Tier", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::False)
        .on_get(get_blockdev_tier);

    let get_all_properties_method = f
        .method("GetAllProperties", (), get_all_properties)
        // a{s(bv)}: Dictionary of property names to tuples
        // In the tuple:
        // b: Indicates whether the property value fetched was successful
        // v: If b is true, represents the value for the given property
        //    If b is false, represents the error returned when fetching the property
        .out_arg(("results", "a{s(bv)}"));

    let get_properties_method = f
        .method("GetProperties", (), get_properties)
        .in_arg(("properties", "as"))
        // a{s(bv)}: Dictionary of property names to tuples
        // In the tuple:
        // b: Indicates whether the property value fetched was successful
        // v: If b is true, represents the value for the given property
        //    If b is false, represents the error returned when fetching the property
        .out_arg(("results", "a{s(bv)}"));

    let object_name = make_object_path(dbus_context);

    let object_path = f
        .object_path(object_name, Some(OPContext::new(parent, uuid)))
        .introspectable()
        .add(
            f.interface(consts::BLOCKDEV_INTERFACE_NAME, ())
                .add_m(set_userid_method)
                .add_p(devnode_property)
                .add_p(hardware_info_property)
                .add_p(initialization_time_property)
                .add_p(pool_property)
                .add_p(tier_property)
                .add_p(user_info_property)
                .add_p(uuid_property),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME, ())
                .add_m(get_all_properties_method)
                .add_m(get_properties_method),
        );

    let path = object_path.get_name().to_owned();
    dbus_context.actions.borrow_mut().push_add(object_path);
    blockdev.set_dbus_path(MaybeDbusPath(Some(path.clone())));
    path
}

fn set_user_info(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let new_id_spec: (bool, &str) = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = (false, uuid_to_string!(DevUuid::nil()));

    let blockdev_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let blockdev_data = get_data!(blockdev_path; default_return; return_message);

    let pool_path = get_parent!(m; blockdev_data; default_return; return_message);
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let (pool_name, pool) = get_mut_pool!(engine; pool_uuid; default_return; return_message);

    let result =
        pool.set_blockdev_user_info(&pool_name, blockdev_data.uuid, tuple_to_option(new_id_spec));

    let msg = match result {
        Ok(RenameAction::NoSource) => {
            let error_message = format!(
                "pool doesn't know about block device {}",
                blockdev_data.uuid
            );
            let (rc, rs) = (DbusErrorEnum::INTERNAL_ERROR as u16, error_message);
            return_message.append3(default_return, rc, rs)
        }
        Ok(RenameAction::Renamed(uuid)) => return_message.append3(
            (true, uuid_to_string!(uuid)),
            msg_code_ok(),
            msg_string_ok(),
        ),
        Ok(RenameAction::Identity) => {
            return_message.append3(default_return, msg_code_ok(), msg_string_ok())
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };

    Ok(vec![msg])
}

fn get_properties_shared(
    m: &MethodInfo<MTFn<TData>, TData>,
    properties: &mut dyn Iterator<Item = String>,
) -> MethodResult {
    let message: &Message = m.msg;
    let object_path = &m.path;

    let return_message = message.method_return();

    let return_value: HashMap<String, (bool, Variant<Box<dyn RefArg>>)> = properties
        .unique()
        .filter_map(|prop| match prop.as_str() {
            consts::BLOCKDEV_TOTAL_SIZE_PROP => Some((
                prop,
                blockdev_operation(m.tree, object_path.get_name(), |_, bd| {
                    Ok((u128::from(*bd.size()) * devicemapper::SECTOR_SIZE as u128).to_string())
                }),
            )),
            _ => None,
        })
        .map(|(key, result)| result_to_tuple(key, result))
        .collect();

    Ok(vec![return_message.append1(return_value)])
}

fn get_all_properties(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    get_properties_shared(
        m,
        &mut vec![consts::BLOCKDEV_TOTAL_SIZE_PROP]
            .into_iter()
            .map(|s| s.to_string()),
    )
}

fn get_properties(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();
    let mut properties: Array<String, _> = get_next_arg(&mut iter, 0)?;
    get_properties_shared(m, &mut properties)
}

/// Perform an operation on a `BlockDev` object for a given
/// DBus implicit argument that is a block device
fn blockdev_operation<F, R>(
    tree: &Tree<MTFn<TData>, TData>,
    object_path: &Path<'static>,
    closure: F,
) -> Result<R, String>
where
    F: Fn(BlockDevTier, &dyn BlockDev) -> Result<R, String>,
    R: dbus::arg::Append,
{
    let dbus_context = tree.get_data();

    let blockdev_path = tree
        .get(object_path)
        .expect("tree must contain implicit argument");

    let blockdev_data = blockdev_path
        .get_data()
        .as_ref()
        .ok_or_else(|| format!("no data for object path {}", object_path))?;

    let pool_path = tree
        .get(&blockdev_data.parent)
        .ok_or_else(|| format!("no path for parent object path {}", &blockdev_data.parent))?;

    let pool_uuid = pool_path
        .get_data()
        .as_ref()
        .ok_or_else(|| format!("no data for object path {}", object_path))?
        .uuid;

    let engine = dbus_context.engine.borrow();
    let (_, pool) = engine
        .get_pool(pool_uuid)
        .ok_or_else(|| format!("no pool corresponding to uuid {}", &pool_uuid))?;
    let (tier, blockdev) = pool
        .get_blockdev(blockdev_data.uuid)
        .ok_or_else(|| format!("no blockdev with uuid {}", blockdev_data.uuid))?;
    closure(tier, blockdev)
}

/// Get a blockdev property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to a
/// blockdev and obtains the property from the blockdev.
fn get_blockdev_property<F, R>(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
    getter: F,
) -> Result<(), MethodErr>
where
    F: Fn(BlockDevTier, &dyn BlockDev) -> Result<R, String>,
    R: dbus::arg::Append,
{
    i.append(
        blockdev_operation(p.tree, p.path.get_name(), getter)
            .map_err(|ref e| MethodErr::failed(e))?,
    );
    Ok(())
}

/// Get the devnode for an object path.
fn get_blockdev_devnode(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |_, p| Ok(format!("{}", p.devnode().display())))
}

fn get_blockdev_hardware_info(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |_, p| {
        Ok(p.hardware_info()
            .map_or_else(|| (false, "".to_owned()), |val| (true, val.to_owned())))
    })
}

fn get_blockdev_user_info(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |_, p| {
        Ok(p.user_info()
            .map_or_else(|| (false, "".to_owned()), |val| (true, val.to_owned())))
    })
}

fn get_blockdev_initialization_time(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |_, p| Ok(p.initialization_time().timestamp() as u64))
}

fn get_blockdev_tier(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |t, _| Ok(t as u16))
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;
use std::path::Path;
use std::vec::Vec;

use dbus;
use dbus::arg::{Array, IterAppend};
use dbus::tree::{
    Access, EmitsChangedSignal, Factory, MTFn, MethodErr, MethodInfo, MethodResult, PropInfo,
};
use dbus::Message;

use uuid::Uuid;

use devicemapper::Sectors;

use crate::dbus_api::consts;
use crate::engine::{BlockDevTier, MaybeDbusPath, Name, Pool, RenameAction};

use crate::dbus_api::blockdev::create_dbus_blockdev;
use crate::dbus_api::filesystem::create_dbus_filesystem;
use crate::dbus_api::types::{DbusContext, DbusErrorEnum, OPContext, TData};
use crate::dbus_api::util::{
    engine_to_dbus_err_tuple, get_next_arg, get_uuid, make_object_path, msg_code_ok, msg_string_ok,
};

fn create_filesystems(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let filesystems: Array<&str, _> = get_next_arg(&mut iter, 0)?;
    let dbus_context = m.tree.get_data();

    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return: Vec<(dbus::Path, &str)> = Vec::new();

    if filesystems.count() > 1 {
        let error_message = "only 1 filesystem per request allowed";
        let (rc, rs) = (DbusErrorEnum::ERROR as u16, error_message);
        return Ok(vec![return_message.append3(default_return, rc, rs)]);
    }

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let (pool_name, pool) = get_mut_pool!(engine; pool_uuid; default_return; return_message);

    let result = pool.create_filesystems(
        pool_uuid,
        &pool_name,
        &filesystems
            .map(|x| (x, None))
            .collect::<Vec<(&str, Option<Sectors>)>>(),
    );

    let msg = match result {
        Ok(ref infos) => {
            let return_value = infos
                .iter()
                .map(|&(name, uuid)| {
                    (
                        // FIXME: To avoid this expect, modify create_filesystem
                        // so that it returns a mutable reference to the
                        // filesystem created.
                        create_dbus_filesystem(
                            dbus_context,
                            object_path.clone(),
                            uuid,
                            pool.get_mut_filesystem(uuid)
                                .expect("just inserted by create_filesystems")
                                .1,
                        ),
                        name,
                    )
                })
                .collect::<Vec<_>>();

            return_message.append3(return_value, msg_code_ok(), msg_string_ok())
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn destroy_filesystems(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let filesystems: Array<dbus::Path<'static>, _> = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return: Vec<&str> = Vec::new();

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let (pool_name, pool) = get_mut_pool!(engine; pool_uuid; default_return; return_message);

    let mut filesystem_map: HashMap<Uuid, dbus::Path<'static>> = HashMap::new();
    for op in filesystems {
        if let Some(filesystem_path) = m.tree.get(&op) {
            let filesystem_uuid = get_data!(filesystem_path; default_return; return_message).uuid;
            filesystem_map.insert(filesystem_uuid, op);
        }
    }

    let result = pool.destroy_filesystems(
        &pool_name,
        &filesystem_map.keys().cloned().collect::<Vec<Uuid>>(),
    );
    let msg = match result {
        Ok(ref uuids) => {
            for uuid in uuids {
                let op = filesystem_map
                    .get(uuid)
                    .expect("'uuids' is a subset of filesystem_map.keys()");
                dbus_context.actions.borrow_mut().push_remove(op, m.tree);
            }

            let return_value: Vec<String> = uuids
                .iter()
                .map(|n| n.to_simple_ref().to_string())
                .collect();
            return_message.append3(return_value, msg_code_ok(), msg_string_ok())
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn snapshot_filesystem(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let filesystem: dbus::Path<'static> = get_next_arg(&mut iter, 0)?;
    let snapshot_name: &str = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = dbus::Path::default();

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let fs_uuid = match m.tree.get(&filesystem) {
        Some(op) => get_data!(op; default_return; return_message).uuid,
        None => {
            let message = format!("no data for object path {}", filesystem);
            let (rc, rs) = (DbusErrorEnum::NOTFOUND as u16, message);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };

    let mut engine = dbus_context.engine.borrow_mut();
    let (pool_name, pool) = get_mut_pool!(engine; pool_uuid; default_return; return_message);

    let msg = match pool.snapshot_filesystem(pool_uuid, &pool_name, fs_uuid, snapshot_name) {
        Ok((uuid, fs)) => {
            let fs_object_path: dbus::Path =
                create_dbus_filesystem(dbus_context, object_path.clone(), uuid, fs);
            return_message.append3(fs_object_path, msg_code_ok(), msg_string_ok())
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };

    Ok(vec![msg])
}

fn add_blockdevs(m: &MethodInfo<MTFn<TData>, TData>, tier: BlockDevTier) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let devs: Array<&str, _> = get_next_arg(&mut iter, 1)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return: Vec<dbus::Path> = Vec::new();

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let (pool_name, pool) = get_mut_pool!(engine; pool_uuid; default_return; return_message);

    let blockdevs = devs.map(|x| Path::new(x)).collect::<Vec<&Path>>();

    let result = pool.add_blockdevs(pool_uuid, &*pool_name, &blockdevs, tier);
    let msg = match result {
        Ok(uuids) => {
            let return_value = uuids
                .iter()
                .map(|uuid| {
                    // FIXME: To avoid this expect, modify add_blockdevs
                    // so that it returns a mutable reference to each
                    // blockdev created.
                    create_dbus_blockdev(
                        dbus_context,
                        object_path.clone(),
                        *uuid,
                        pool.get_mut_blockdev(*uuid)
                            .expect("just inserted by add_blockdevs")
                            .1,
                    )
                })
                .collect::<Vec<_>>();

            return_message.append3(return_value, msg_code_ok(), msg_string_ok())
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };

    Ok(vec![msg])
}

fn add_datadevs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    add_blockdevs(m, BlockDevTier::Data)
}

fn add_cachedevs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    add_blockdevs(m, BlockDevTier::Cache)
}

fn rename_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let new_name: &str = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = false;

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let msg = match dbus_context
        .engine
        .borrow_mut()
        .rename_pool(pool_uuid, new_name)
    {
        Ok(RenameAction::NoSource) => {
            let error_message = format!("engine doesn't know about pool {}", &pool_uuid);
            let (rc, rs) = (DbusErrorEnum::INTERNAL_ERROR as u16, error_message);
            return_message.append3(default_return, rc, rs)
        }
        Ok(RenameAction::Identity) => return_message.append3(false, msg_code_ok(), msg_string_ok()),
        Ok(RenameAction::Renamed) => return_message.append3(true, msg_code_ok(), msg_string_ok()),
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

/// Get a pool property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to a
/// Pool and obtains the property from the pool.
fn get_pool_property<F, R>(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
    getter: F,
) -> Result<(), MethodErr>
where
    F: Fn((Name, Uuid, &Pool)) -> Result<R, MethodErr>,
    R: dbus::arg::Append,
{
    let dbus_context = p.tree.get_data();
    let object_path = p.path.get_name();
    let pool_path = p
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");

    let pool_uuid = pool_path
        .get_data()
        .as_ref()
        .ok_or_else(|| MethodErr::failed(&format!("no data for object path {}", object_path)))?
        .uuid;

    let engine = dbus_context.engine.borrow();
    let (pool_name, pool) = engine.get_pool(pool_uuid).ok_or_else(|| {
        MethodErr::failed(&format!("no pool corresponding to uuid {}", &pool_uuid))
    })?;

    i.append(getter((pool_name, pool_uuid, pool))?);
    Ok(())
}

fn get_pool_name(i: &mut IterAppend, p: &PropInfo<MTFn<TData>, TData>) -> Result<(), MethodErr> {
    get_pool_property(i, p, |(name, _, _)| Ok(name.to_owned()))
}

fn get_pool_total_physical_used(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    fn get_used((_, uuid, pool): (Name, Uuid, &Pool)) -> Result<String, MethodErr> {
        let err_func = |_| {
            MethodErr::failed(&format!(
                "no total physical size computed for pool with uuid {}",
                uuid
            ))
        };

        pool.total_physical_used()
            .map(|u| Ok(format!("{}", *u)))
            .map_err(err_func)?
    }

    get_pool_property(i, p, get_used)
}

fn get_pool_total_physical_size(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_pool_property(i, p, |(_, _, p)| {
        Ok(format!("{}", *p.total_physical_size()))
    })
}

fn get_pool_state(i: &mut IterAppend, p: &PropInfo<MTFn<TData>, TData>) -> Result<(), MethodErr> {
    get_pool_property(i, p, |(_, _, pool)| Ok(pool.state() as u16))
}

fn get_pool_extend_state(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_pool_property(i, p, |(_, _, pool)| Ok(pool.extend_state() as u16))
}

fn get_space_state(i: &mut IterAppend, p: &PropInfo<MTFn<TData>, TData>) -> Result<(), MethodErr> {
    get_pool_property(i, p, |(_, _, pool)| Ok(pool.free_space_state() as u16))
}

pub fn create_dbus_pool<'a>(
    dbus_context: &DbusContext,
    parent: dbus::Path<'static>,
    uuid: Uuid,
    pool: &mut Pool,
) -> dbus::Path<'a> {
    let f = Factory::new_fn();

    let create_filesystems_method = f
        .method("CreateFilesystems", (), create_filesystems)
        .in_arg(("specs", "as"))
        .out_arg(("filesystems", "a(os)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let destroy_filesystems_method = f
        .method("DestroyFilesystems", (), destroy_filesystems)
        .in_arg(("filesystems", "ao"))
        .out_arg(("results", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let add_blockdevs_method = f
        .method("AddDataDevs", (), add_datadevs)
        .in_arg(("devices", "as"))
        .out_arg(("results", "ao"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let add_cachedevs_method = f
        .method("AddCacheDevs", (), add_cachedevs)
        .in_arg(("devices", "as"))
        .out_arg(("results", "ao"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let rename_method = f
        .method("SetName", (), rename_pool)
        .in_arg(("name", "s"))
        .out_arg(("action", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let snapshot_method = f
        .method("SnapshotFilesystem", (), snapshot_filesystem)
        .in_arg(("origin", "o"))
        .in_arg(("snapshot_name", "s"))
        .out_arg(("result", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let name_property = f
        .property::<&str, _>(consts::POOL_NAME_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_pool_name);

    let total_physical_size_property = f
        .property::<&str, _>("TotalPhysicalSize", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::False)
        .on_get(get_pool_total_physical_size);

    let total_physical_used_property = f
        .property::<&str, _>("TotalPhysicalUsed", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::False)
        .on_get(get_pool_total_physical_used);

    let uuid_property = f
        .property::<&str, _>("Uuid", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_uuid);

    let state_property = f
        .property::<u16, _>(consts::POOL_STATE_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_pool_state);

    let extend_state_property = f
        .property::<u16, _>(consts::POOL_EXTEND_STATE_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_pool_extend_state);

    let space_state_property = f
        .property::<u16, _>(consts::POOL_SPACE_STATE_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_space_state);

    let object_name = make_object_path(dbus_context);

    let object_path = f
        .object_path(object_name, Some(OPContext::new(parent, uuid)))
        .introspectable()
        .add(
            f.interface(consts::POOL_INTERFACE_NAME, ())
                .add_m(create_filesystems_method)
                .add_m(destroy_filesystems_method)
                .add_m(snapshot_method)
                .add_m(add_blockdevs_method)
                .add_m(add_cachedevs_method)
                .add_m(rename_method)
                .add_p(name_property)
                .add_p(total_physical_size_property)
                .add_p(total_physical_used_property)
                .add_p(uuid_property)
                .add_p(state_property)
                .add_p(space_state_property)
                .add_p(extend_state_property),
        );

    let path = object_path.get_name().to_owned();
    dbus_context.actions.borrow_mut().push_add(object_path);
    pool.set_dbus_path(MaybeDbusPath(Some(path.clone())));
    path
}

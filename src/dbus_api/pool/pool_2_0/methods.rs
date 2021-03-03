// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use dbus::{arg::Array, Message};
use dbus_tree::{MTSync, MethodInfo, MethodResult};
use devicemapper::Sectors;

use crate::{
    dbus_api::{
        consts::filesystem_interface_list,
        filesystem::create_dbus_filesystem,
        pool::shared::{add_blockdevs, BlockDevOp},
        types::{DbusErrorEnum, TData},
        util::{engine_to_dbus_err_tuple, get_next_arg, msg_code_ok, msg_string_ok},
    },
    engine::{CreateAction, EngineAction, FilesystemUuid, Name, PoolUuid, RenameAction},
};

pub fn create_filesystems(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let filesystems: Array<&str, _> = get_next_arg(&mut iter, 0)?;
    let dbus_context = m.tree.get_data();

    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return: (bool, Vec<(dbus::Path, &str)>) = (false, Vec::new());

    if filesystems.count() > 1 {
        let error_message = "only 1 filesystem per request allowed";
        let (rc, rs) = (DbusErrorEnum::ERROR as u16, error_message);
        return Ok(vec![return_message.append3(default_return, rc, rs)]);
    }

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = typed_uuid!(
        get_data!(pool_path; default_return; return_message).uuid;
        Pool;
        default_return;
        return_message
    );

    let mut mutex_lock = mutex_lock!(dbus_context.engine);
    let (pool_name, pool) = get_mut_pool!(mutex_lock; pool_uuid; default_return; return_message);

    let result = log_action!(pool.create_filesystems(
        pool_uuid,
        &filesystems
            .map(|x| (x, None))
            .collect::<Vec<(&str, Option<Sectors>)>>(),
    ));

    let infos = match result {
        Ok(created_set) => created_set.changed(),
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };

    let return_value = match infos {
        Some(ref newly_created_filesystems) => {
            let v = newly_created_filesystems
                .iter()
                .map(|&(name, uuid)| {
                    let filesystem = pool.get_filesystem(uuid)
                        .expect("just inserted by create_filesystems")
                        .1;
                    // FIXME: To avoid this expect, modify create_filesystem
                    // so that it returns a mutable reference to the
                    // filesystem created.
                    let info = (
                        create_dbus_filesystem(
                            dbus_context,
                            object_path.clone(),
                            &pool_name,
                            &Name::new(name.to_string()),
                            uuid,
                            filesystem,
                        ),
                        name,
                    );
                    if let Err(e) = filesystem.send_udev_change() {
                        warn!("Failed to send a synthetic udev event after filesystem creation for udev rule: {}", e);
                    }
                    info
                })
                .collect::<Vec<_>>();
            (true, v)
        }
        None => default_return,
    };

    Ok(vec![return_message.append3(
        return_value,
        msg_code_ok(),
        msg_string_ok(),
    )])
}

pub fn destroy_filesystems(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let filesystems: Array<dbus::Path<'static>, _> = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return: (bool, Vec<String>) = (false, Vec::new());

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = typed_uuid!(
        get_data!(pool_path; default_return; return_message).uuid;
        Pool;
        default_return;
        return_message
    );

    let mut mutex_lock = mutex_lock!(dbus_context.engine);
    let (pool_name, pool) = get_mut_pool!(mutex_lock; pool_uuid; default_return; return_message);

    let mut filesystem_map: HashMap<FilesystemUuid, dbus::Path<'static>> = HashMap::new();
    for path in filesystems {
        if let Some((u, path)) = m.tree.get(&path).and_then(|op| {
            op.get_data()
                .as_ref()
                .map(|d| (&d.uuid, op.get_name().clone()))
        }) {
            let uuid = *typed_uuid!(u; Fs; default_return; return_message);
            filesystem_map.insert(uuid, path);
        }
    }

    let result = log_action!(pool.destroy_filesystems(
        &pool_name,
        &filesystem_map.keys().cloned().collect::<Vec<_>>(),
    ));
    let msg = match result {
        Ok(uuids) => {
            // Only get changed values here as non-existant filesystems will have been filtered out
            // before calling destroy_filesystems
            let uuid_vec: Vec<String> = if let Some(ref changed_uuids) = uuids.changed() {
                for uuid in changed_uuids {
                    let op = filesystem_map
                        .get(uuid)
                        .expect("'uuids' is a subset of filesystem_map.keys()");
                    dbus_context.push_remove(op, filesystem_interface_list());
                }
                changed_uuids
                    .iter()
                    .map(|uuid| uuid_to_string!(uuid))
                    .collect()
            } else {
                Vec::new()
            };
            return_message.append3((true, uuid_vec), msg_code_ok(), msg_string_ok())
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn snapshot_filesystem(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let filesystem: dbus::Path<'static> = get_next_arg(&mut iter, 0)?;
    let snapshot_name: &str = get_next_arg(&mut iter, 1)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = (false, dbus::Path::default());

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = typed_uuid!(
        get_data!(pool_path; default_return; return_message).uuid;
        Pool;
        default_return;
        return_message
    );

    let fs_uuid = match m.tree.get(&filesystem) {
        Some(op) => typed_uuid!(
            get_data!(op; default_return; return_message).uuid;
            Fs;
            default_return;
            return_message
        ),
        None => {
            let message = format!("no data for object path {}", filesystem);
            let (rc, rs) = (DbusErrorEnum::NOTFOUND as u16, message);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };

    let mut mutex_lock = mutex_lock!(dbus_context.engine);
    let (pool_name, pool) = get_mut_pool!(mutex_lock; pool_uuid; default_return; return_message);

    let msg = match log_action!(pool.snapshot_filesystem(pool_uuid, fs_uuid, snapshot_name)) {
        Ok(CreateAction::Created((uuid, fs))) => {
            let fs_object_path: dbus::Path = create_dbus_filesystem(
                dbus_context,
                object_path.clone(),
                &pool_name,
                &Name::new(snapshot_name.to_string()),
                uuid,
                fs,
            );
            return_message.append3((true, fs_object_path), msg_code_ok(), msg_string_ok())
        }
        Ok(CreateAction::Identity) => {
            return_message.append3(default_return, msg_code_ok(), msg_string_ok())
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };

    Ok(vec![msg])
}

pub fn add_datadevs(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    add_blockdevs(m, BlockDevOp::AddData)
}

/// This method supports a method for adding cachedevs to a pool where,
/// if there was no previously existing cache, the addition of the cache
/// devices caused a cache to be automatically constructed. The newer
/// version of the interface requires initializing the cache in a distinct
/// step and the engine's Pool trait is designed to accomodate that
/// interface. For this reason, this method contains an extra step:
/// it must determine whether or not the cache is already initialized in
/// order to specify which Pool trait method must be invoked.
pub fn add_cachedevs(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let return_message = message.method_return();
    let object_path = m.path.get_name();
    let default_return: (bool, Vec<dbus::Path>) = (false, Vec::new());
    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = typed_uuid!(
        get_data!(pool_path; default_return; return_message).uuid;
        Pool;
        default_return;
        return_message
    );
    let cache_initialized = {
        let dbus_context = m.tree.get_data();
        let mutex_lock = mutex_lock!(dbus_context.engine);
        let (_, pool) = get_pool!(mutex_lock; pool_uuid; default_return; return_message);
        pool.has_cache()
    };
    add_blockdevs(
        m,
        if cache_initialized {
            BlockDevOp::AddCache
        } else {
            BlockDevOp::InitCache
        },
    )
}

pub fn rename_pool(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let new_name: &str = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = (false, uuid_to_string!(PoolUuid::nil()));

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = typed_uuid!(
        get_data!(pool_path; default_return; return_message).uuid;
        Pool;
        default_return;
        return_message
    );

    let msg = match log_action!(mutex_lock!(dbus_context.engine).rename_pool(pool_uuid, new_name)) {
        Ok(RenameAction::NoSource) => {
            let error_message = format!("engine doesn't know about pool {}", pool_uuid);
            let (rc, rs) = (DbusErrorEnum::INTERNAL_ERROR as u16, error_message);
            return_message.append3(default_return, rc, rs)
        }
        Ok(RenameAction::Identity) => {
            return_message.append3(default_return, msg_code_ok(), msg_string_ok())
        }
        Ok(RenameAction::Renamed(uuid)) => {
            dbus_context.push_pool_name_change(object_path, new_name, m.tree);
            return_message.append3(
                (true, uuid_to_string!(uuid)),
                msg_code_ok(),
                msg_string_ok(),
            )
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

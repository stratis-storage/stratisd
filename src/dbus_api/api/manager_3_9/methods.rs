// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{arg::OwnedFd, Message};
use dbus_tree::{MTSync, MethodInfo, MethodResult};
use futures::executor::block_on;

use crate::{
    dbus_api::{
        blockdev::create_dbus_blockdev,
        filesystem::create_dbus_filesystem,
        pool::create_dbus_pool,
        types::{DbusErrorEnum, TData, OK_STRING},
        util::{engine_to_dbus_err_tuple, get_next_arg, tuple_to_option},
    },
    engine::{Name, PoolIdentifier, PoolUuid, StartAction, TokenUnlockMethod},
    stratis::StratisError,
};

pub fn start_pool(m: &MethodInfo<'_, MTSync<TData>, TData>) -> MethodResult {
    let base_path = m.path.get_name();
    let message: &Message = m.msg;
    let mut iter = message.iter_init();
    let dbus_context = m.tree.get_data();
    let default_return: (
        bool,
        (
            dbus::Path<'static>,
            Vec<dbus::Path<'static>>,
            Vec<dbus::Path<'static>>,
        ),
    ) = (false, (dbus::Path::default(), Vec::new(), Vec::new()));
    let return_message = message.method_return();

    let id_str: &str = get_next_arg(&mut iter, 0)?;
    let id = {
        let id_type_str: &str = get_next_arg(&mut iter, 1)?;
        match id_type_str {
            "uuid" => match PoolUuid::parse_str(id_str) {
                Ok(u) => PoolIdentifier::Uuid(u),
                Err(e) => {
                    let (rc, rs) = engine_to_dbus_err_tuple(&e);
                    return Ok(vec![return_message.append3(default_return, rc, rs)]);
                }
            },
            "name" => PoolIdentifier::Name(Name::new(id_str.to_string())),
            _ => {
                let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Msg(format!(
                    "ID type {id_type_str} not recognized"
                )));
                return Ok(vec![return_message.append3(default_return, rc, rs)]);
            }
        }
    };
    let unlock_method_tup: (bool, (bool, u32)) = get_next_arg(&mut iter, 2)?;
    let unlock_method =
        TokenUnlockMethod::from_options(tuple_to_option(unlock_method_tup).map(tuple_to_option));
    let fd_opt: (bool, OwnedFd) = get_next_arg(&mut iter, 3)?;
    let fd = tuple_to_option(fd_opt);
    let remove_cache: bool = get_next_arg(&mut iter, 4)?;

    let ret = match handle_action!(block_on(dbus_context.engine.start_pool(
        id.clone(),
        unlock_method,
        fd.map(|f| f.into_fd()),
        remove_cache,
    ))) {
        Ok(StartAction::Started(_)) => {
            let guard = match block_on(dbus_context.engine.get_pool(id.clone())) {
                Some(g) => g,
                None => {
                    let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Msg(
                        format!("Pool with {id:?} was successfully started but appears to have been removed before it could be exposed on the D-Bus")
                    ));
                    return Ok(vec![return_message.append3(default_return, rc, rs)]);
                }
            };

            let (pool_name, pool_uuid, pool) = guard.as_tuple();
            let pool_path =
                create_dbus_pool(dbus_context, base_path.clone(), &pool_name, pool_uuid, pool);
            let mut bd_paths = Vec::new();
            for (bd_uuid, tier, bd) in pool.blockdevs() {
                bd_paths.push(create_dbus_blockdev(
                    dbus_context,
                    pool_path.clone(),
                    bd_uuid,
                    tier,
                    bd,
                ));
            }
            let mut fs_paths = Vec::new();
            for (name, fs_uuid, fs) in pool.filesystems() {
                fs_paths.push(create_dbus_filesystem(
                    dbus_context,
                    pool_path.clone(),
                    &pool_name,
                    &name,
                    fs_uuid,
                    fs,
                ));
            }

            if pool.is_encrypted() {
                dbus_context.push_locked_pools(block_on(dbus_context.engine.locked_pools()));
            }
            dbus_context.push_stopped_pools(block_on(dbus_context.engine.stopped_pools()));

            (true, (pool_path, bd_paths, fs_paths))
        }
        Ok(StartAction::Identity) => default_return,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };

    Ok(vec![return_message.append3(
        ret,
        DbusErrorEnum::OK as u16,
        OK_STRING.to_string(),
    )])
}

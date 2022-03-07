// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::convert::TryFrom;

use dbus::{Message, Path};
use dbus_tree::{MTSync, MethodInfo, MethodResult};
use futures::executor::block_on;

use crate::{
    dbus_api::{
        blockdev::create_dbus_blockdev,
        consts,
        filesystem::create_dbus_filesystem,
        pool::create_dbus_pool,
        types::{DbusErrorEnum, TData, OK_STRING},
        util::{engine_to_dbus_err_tuple, get_next_arg, tuple_to_option},
    },
    engine::{Engine, LockKey, Pool, PoolUuid, StartAction, StopAction, UnlockMethod},
    stratis::StratisError,
};

pub fn start_pool<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> MethodResult
where
    E: 'static + Engine,
{
    let base_path = m.path.get_name();
    let message: &Message = m.msg;
    let mut iter = message.iter_init();
    let dbus_context = m.tree.get_data();
    let default_return: (
        bool,
        (Path<'static>, Vec<Path<'static>>, Vec<Path<'static>>),
    ) = (false, (Path::default(), Vec::new(), Vec::new()));
    let return_message = message.method_return();

    let pool_uuid_str: &str = get_next_arg(&mut iter, 0)?;
    let pool_uuid = match PoolUuid::parse_str(pool_uuid_str) {
        Ok(uuid) => uuid,
        Err(e) => {
            let e = StratisError::Chained(
                "Malformed UUID passed to StartPool".to_string(),
                Box::new(e),
            );
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };
    let unlock_method = {
        let unlock_method_tup: (bool, &str) = get_next_arg(&mut iter, 1)?;
        match tuple_to_option(unlock_method_tup) {
            Some(unlock_method_str) => match UnlockMethod::try_from(unlock_method_str) {
                Ok(um) => Some(um),
                Err(e) => {
                    let (rc, rs) = engine_to_dbus_err_tuple(&e);
                    return Ok(vec![return_message.append3(default_return, rc, rs)]);
                }
            },
            None => None,
        }
    };

    let ret = match handle_action!(block_on(
        dbus_context.engine.start_pool(pool_uuid, unlock_method,)
    )) {
        Ok(StartAction::Started(_)) => {
            let guard = match block_on(dbus_context.engine.get_pool(LockKey::Uuid(pool_uuid))) {
                Some(g) => g,
                None => {
                    let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Msg(
                        format!("Pool with UUID {} was successfully started but appears to have been removed before it could be exposed on the D-Bus", pool_uuid)
                    ));
                    return Ok(vec![return_message.append3(default_return, rc, rs)]);
                }
            };

            let (pool_name, _, pool) = guard.as_tuple();
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

pub fn stop_pool<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> MethodResult
where
    E: 'static + Engine,
{
    let message: &Message = m.msg;
    let mut iter = message.iter_init();
    let dbus_context = m.tree.get_data();
    let default_return = (false, String::new());
    let return_message = message.method_return();

    let pool_path: dbus::Path<'static> = get_next_arg(&mut iter, 0)?;
    let pool_uuid = match m
        .tree
        .get(&pool_path)
        .and_then(|op| op.get_data().as_ref())
        .map(|d| &d.uuid)
    {
        Some(uuid) => *typed_uuid!(uuid; Pool; default_return; return_message),
        None => {
            return Ok(vec![return_message.append3(
                default_return,
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            )]);
        }
    };

    // If Some(_), send a locked pool property change signal only if the pool is
    // encrypted. If None, the pool may already be stopped or not exist at all.
    // Both of these cases are handled by stop_pool and the value we provide
    // for send_locked_signal does not matter as send_locked_signal is only
    // used when a pool is newly stopped which can only occur if the pool is found
    // here.
    let send_locked_signal = block_on(dbus_context.engine.get_pool(LockKey::Uuid(pool_uuid)))
        .map(|g| {
            let (_, _, p) = g.as_tuple();
            p.is_encrypted()
        })
        .unwrap_or(false);

    let msg = match handle_action!(block_on(dbus_context.engine.stop_pool(pool_uuid))) {
        Ok(StopAction::Stopped(_)) => {
            dbus_context.push_remove(&pool_path, consts::pool_interface_list());
            if send_locked_signal {
                dbus_context.push_locked_pools(block_on(dbus_context.engine.locked_pools()));
            }
            dbus_context.push_stopped_pools(block_on(dbus_context.engine.stopped_pools()));
            return_message.append3(
                (true, uuid_to_string!(pool_uuid)),
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            )
        }
        Ok(StopAction::Identity) => return_message.append3(
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };

    Ok(vec![msg])
}

pub fn refresh_state<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> MethodResult
where
    E: 'static + Engine,
{
    let message: &Message = m.msg;
    let dbus_context = m.tree.get_data();
    let return_message = message.method_return();

    let msg = match block_on(dbus_context.engine.refresh_state()) {
        Ok(()) => return_message.append2(DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return Ok(vec![return_message.append2(rc, rs)]);
        }
    };

    Ok(vec![msg])
}

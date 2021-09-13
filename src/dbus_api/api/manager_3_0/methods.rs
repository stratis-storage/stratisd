// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{convert::TryFrom, os::unix::io::AsRawFd, path::Path, time::Duration};

use dbus::{
    arg::{Array, OwnedFd},
    Message,
};
use dbus_tree::{MTSync, MethodInfo, MethodResult};
use futures::executor::block_on;

use crate::{
    dbus_api::{
        consts,
        types::{DbusErrorEnum, TData, OK_STRING},
        util::{engine_to_dbus_err_tuple, get_next_arg, tuple_to_option},
        POOL_CONDVAR, POOL_SETUP_STATE,
    },
    engine::{
        CreateAction, DeleteAction, EncryptionInfo, Engine, EngineAction, KeyActions,
        KeyDescription, MappingCreateAction, MappingDeleteAction, PoolUuid, UnlockMethod,
    },
    stratis::StratisError,
};

type EncryptionParams = (Option<(bool, String)>, Option<(bool, (String, String))>);

pub fn destroy_pool<E>(m: &MethodInfo<MTSync<TData<E>>, TData<E>>) -> MethodResult
where
    E: 'static + Engine,
{
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let pool_path: dbus::Path<'static> = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();

    let default_return = (false, uuid_to_string!(PoolUuid::nil()));
    let return_message = message.method_return();

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

    let msg = match handle_action!(block_on(dbus_context.engine.destroy_pool(pool_uuid))) {
        Ok(DeleteAction::Deleted(uuid)) => {
            dbus_context.push_remove(&pool_path, consts::pool_interface_list());
            return_message.append3(
                (true, uuid_to_string!(uuid)),
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            )
        }
        Ok(DeleteAction::Identity) => return_message.append3(
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn unset_key<E>(m: &MethodInfo<MTSync<TData<E>>, TData<E>>) -> MethodResult
where
    E: 'static + Engine,
{
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let key_desc_str: String = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let default_return = false;
    let return_message = message.method_return();

    let msg = match handle_action!(block_on(dbus_context.engine.get_key_handler_mut()).unset(
        &match KeyDescription::try_from(key_desc_str) {
            Ok(kd) => kd,
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                return Ok(vec![return_message.append3(default_return, rc, rs)]);
            }
        }
    )) {
        Ok(idem_resp) => {
            let return_value = matches!(idem_resp, MappingDeleteAction::Deleted(_));
            return_message.append3(
                return_value,
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            )
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn set_key<E>(m: &MethodInfo<MTSync<TData<E>>, TData<E>>) -> MethodResult
where
    E: 'static + Engine,
{
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let key_desc_str: String = get_next_arg(&mut iter, 0)?;
    let key_fd: OwnedFd = get_next_arg(&mut iter, 1)?;

    let dbus_context = m.tree.get_data();
    let default_return = (false, false);
    let return_message = message.method_return();

    let msg = match handle_action!(block_on(dbus_context.engine.get_key_handler_mut()).set(
        &match KeyDescription::try_from(key_desc_str) {
            Ok(kd) => kd,
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                return Ok(vec![return_message.append3(default_return, rc, rs)]);
            }
        },
        key_fd.as_raw_fd(),
    )) {
        Ok(idem_resp) => {
            let return_value = match idem_resp {
                MappingCreateAction::Created(_) => (true, false),
                MappingCreateAction::ValueChanged(_) => (true, true),
                MappingCreateAction::Identity => default_return,
            };
            return_message.append3(
                return_value,
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            )
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn unlock_pool<E>(m: &MethodInfo<MTSync<TData<E>>, TData<E>>) -> MethodResult
where
    E: 'static + Engine,
{
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let dbus_context = m.tree.get_data();
    let default_return: (_, Vec<String>) = (false, Vec::new());
    let return_message = message.method_return();

    let pool_uuid_str: &str = get_next_arg(&mut iter, 0)?;
    let pool_uuid_result = PoolUuid::parse_str(pool_uuid_str);
    let pool_uuid = match pool_uuid_result {
        Ok(uuid) => uuid,
        Err(e) => {
            let e = StratisError::Chained(
                "Malformed UUID passed to UnlockPool".to_string(),
                Box::new(e),
            );
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };
    let unlock_method = {
        let unlock_method_str: &str = get_next_arg(&mut iter, 1)?;
        match UnlockMethod::try_from(unlock_method_str) {
            Ok(um) => um,
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                return Ok(vec![return_message.append3(default_return, rc, rs)]);
            }
        }
    };

    let msg = match handle_action!(block_on(
        dbus_context.engine.unlock_pool(pool_uuid, unlock_method)
    )) {
        Ok(unlock_action) => match unlock_action.changed() {
            Some(vec) => {
                let str_uuids: Vec<_> = vec.into_iter().map(|u| uuid_to_string!(u)).collect();
                return_message.append3(
                    (true, str_uuids),
                    DbusErrorEnum::OK as u16,
                    OK_STRING.to_string(),
                )
            }
            None => return_message.append3(
                default_return,
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            ),
        },
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn engine_state_report<E>(m: &MethodInfo<MTSync<TData<E>>, TData<E>>) -> MethodResult
where
    E: 'static + Engine,
{
    let message: &Message = m.msg;

    let return_message = message.method_return();
    let default_return = String::new();

    let dbus_context = m.tree.get_data();

    let msg = match serde_json::to_string(&dbus_context.engine.engine_state_report()) {
        Ok(string) => {
            return_message.append3(string, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e.into());
            return_message.append3(default_return, rc, rs)
        }
    };

    Ok(vec![msg])
}

pub fn create_pool<E>(m: &MethodInfo<MTSync<TData<E>>, TData<E>>) -> MethodResult
where
    E: 'static + Engine,
{
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let name: &str = get_next_arg(&mut iter, 0)?;
    let redundancy_tuple: (bool, u16) = get_next_arg(&mut iter, 1)?;
    let devs: Array<&str, _> = get_next_arg(&mut iter, 2)?;
    let (key_desc_tuple, clevis_tuple): EncryptionParams = (
        Some(get_next_arg(&mut iter, 3)?),
        Some(get_next_arg(&mut iter, 4)?),
    );

    let return_message = message.method_return();

    let default_return: (bool, (dbus::Path<'static>, Vec<dbus::Path<'static>>)) =
        (false, (dbus::Path::default(), Vec::new()));

    let key_desc = match key_desc_tuple.and_then(tuple_to_option) {
        Some(kds) => match KeyDescription::try_from(kds) {
            Ok(kd) => Some(kd),
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                return Ok(vec![return_message.append3(default_return, rc, rs)]);
            }
        },
        None => None,
    };

    let clevis_info = match clevis_tuple.and_then(tuple_to_option) {
        Some((pin, json_string)) => match serde_json::from_str(json_string.as_str()) {
            Ok(j) => Some((pin, j)),
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Serde(e));
                return Ok(vec![return_message.append3(default_return, rc, rs)]);
            }
        },
        None => None,
    };

    let dbus_context = m.tree.get_data();
    let result = handle_action!(block_on(dbus_context.engine.create_pool(
        name,
        &devs.map(|x| Path::new(x)).collect::<Vec<_>>(),
        tuple_to_option(redundancy_tuple),
        EncryptionInfo::from_options((key_desc, clevis_info)).as_ref(),
    )));

    let msg = match result {
        Ok(pool_uuid_action) => {
            let results = match pool_uuid_action {
                CreateAction::Created(uuid) => {
                    let mut guard = pool_notify_lock!(
                        (*POOL_SETUP_STATE).lock(),
                        return_message,
                        default_return
                    );
                    guard.insert(uuid, None);
                    // NOTE: Condvar keeps guard acquired if condition is false so we do not
                    // need to check again as nothing can change the state between these two
                    // statements.
                    let (mut guard, timeout) = pool_notify_lock!(
                        (*POOL_CONDVAR).wait_timeout_while(
                            guard,
                            Duration::from_secs(120),
                            |state| {
                                if let Some(paths) = state.get(&uuid) {
                                    paths.is_none()
                                } else {
                                    // End wait if pool is not in state so that we can return an
                                    // error.
                                    false
                                }
                            }
                        ),
                        return_message,
                        default_return
                    );
                    if timeout.timed_out() {
                        warn!("Create pool request timed out waiting for pool to be created");
                    }
                    if let Some(Some((pool_path, bd_paths))) = guard.remove(&uuid) {
                        (true, (pool_path, bd_paths))
                    } else {
                        let err = StratisError::Msg(format!(
                            "Pool with UUID {} was not found after creation was requested",
                            uuid
                        ));
                        let (rc, rs) = engine_to_dbus_err_tuple(&err);
                        return Ok(vec![return_message.append3(default_return, rc, rs)]);
                    }
                }
                CreateAction::Identity => default_return,
            };
            return_message.append3(results, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Err(x) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&x);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

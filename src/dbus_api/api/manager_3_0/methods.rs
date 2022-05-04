// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{convert::TryFrom, os::unix::io::AsRawFd, path::Path};

use dbus::{
    arg::{Array, OwnedFd},
    Message,
};
use dbus_tree::{MTSync, MethodInfo, MethodResult};
use futures::executor::block_on;

use crate::{
    dbus_api::{
        blockdev::create_dbus_blockdev,
        consts,
        pool::create_dbus_pool,
        types::{DbusErrorEnum, TData, OK_STRING},
        util::{engine_to_dbus_err_tuple, get_next_arg, tuple_to_option},
    },
    engine::{
        CreateAction, DeleteAction, EncryptionInfo, Engine, EngineAction, KeyActions,
        KeyDescription, LockKey, MappingCreateAction, MappingDeleteAction, Pool, PoolUuid,
        UnlockMethod,
    },
    stratis::StratisError,
};

type EncryptionParams = (Option<(bool, String)>, Option<(bool, (String, String))>);

pub fn destroy_pool<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> MethodResult
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

pub fn unset_key<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> MethodResult
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

pub fn set_key<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> MethodResult
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

pub fn unlock_pool<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> MethodResult
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

pub fn engine_state_report<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> MethodResult
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

pub fn create_pool<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> MethodResult
where
    E: 'static + Engine,
{
    let base_path = m.path.get_name();
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let name: &str = get_next_arg(&mut iter, 0)?;
    let redundancy_tuple: (bool, u16) = get_next_arg(&mut iter, 1)?;
    let devs: Array<'_, &str, _> = get_next_arg(&mut iter, 2)?;
    let (key_desc_tuple, clevis_tuple): EncryptionParams = (
        Some(get_next_arg(&mut iter, 3)?),
        Some(get_next_arg(&mut iter, 4)?),
    );

    let return_message = message.method_return();

    let default_return: (bool, (dbus::Path<'static>, Vec<dbus::Path<'static>>)) =
        (false, (dbus::Path::default(), Vec::new()));

    match tuple_to_option(redundancy_tuple) {
        None | Some(0) => {}
        Some(n) => {
            return Ok(vec![return_message.append3(
                default_return,
                1u16,
                format!("code {} does not correspond to any redundancy", n),
            )]);
        }
    }

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
    let create_result = handle_action!(block_on(dbus_context.engine.create_pool(
        name,
        &devs.map(Path::new).collect::<Vec<&Path>>(),
        EncryptionInfo::from_options((key_desc, clevis_info)).as_ref(),
    )));
    match create_result {
        Ok(pool_uuid_action) => match pool_uuid_action {
            CreateAction::Created(uuid) => {
                let guard = match block_on(dbus_context.engine.get_pool(LockKey::Uuid(uuid))) {
                    Some(g) => g,
                    None => {
                        let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Msg(
                            format!("Pool with UUID {} was successfully started but appears to have been removed before it could be exposed on the D-Bus", uuid)
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

                Ok(vec![return_message.append3(
                    (true, (pool_path, bd_paths)),
                    DbusErrorEnum::OK as u16,
                    OK_STRING.to_string(),
                )])
            }
            CreateAction::Identity => Ok(vec![return_message.append3(
                default_return,
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            )]),
        },
        Err(x) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&x);
            Ok(vec![return_message.append3(default_return, rc, rs)])
        }
    }
}

pub fn list_keys<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> MethodResult
where
    E: 'static + Engine,
{
    let message: &Message = m.msg;

    let return_message = message.method_return();

    let default_return: Vec<String> = Vec::new();
    let dbus_context = m.tree.get_data();

    Ok(vec![
        match block_on(dbus_context.engine.get_key_handler()).list() {
            Ok(keys) => {
                let key_strings = keys
                    .into_iter()
                    .map(|k| k.as_application_str().to_string())
                    .collect::<Vec<_>>();
                return_message.append3(key_strings, DbusErrorEnum::OK as u16, OK_STRING.to_string())
            }
            Err(x) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&x);
                return_message.append3(default_return, rc, rs)
            }
        },
    ])
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, convert::TryFrom, os::unix::io::AsRawFd, path::Path, vec::Vec};

use dbus::{
    arg::{Array, OwnedFd},
    Message,
};
use dbus_tree::{MTSync, MethodInfo, MethodResult};

use crate::{
    dbus_api::{
        blockdev::create_dbus_blockdev,
        pool::create_dbus_pool,
        types::{CreatePoolParams, TData},
        util::{
            engine_to_dbus_err_tuple, get_next_arg, msg_code_ok, msg_string_ok, tuple_to_option,
        },
    },
    engine::{
        CreateAction, EncryptionInfo, EngineAction, KeyDescription, MappingCreateAction, Name,
        PoolUuid, UnlockMethod,
    },
    stratis::{ErrorEnum, StratisError},
};

type EncryptionParams = (Option<(bool, String)>, Option<(bool, (String, String))>);

/// Shared code for the creation of pools using the D-Bus API without the option
/// for a key description or with an optional key description in later versions of
/// the interface.
pub fn create_pool_shared(
    m: &MethodInfo<MTSync<TData>, TData>,
    has_additional_params: CreatePoolParams,
) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let name: &str = get_next_arg(&mut iter, 0)?;
    let redundancy_tuple: (bool, u16) = get_next_arg(&mut iter, 1)?;
    let devs: Array<&str, _> = get_next_arg(&mut iter, 2)?;
    let (key_desc_tuple, clevis_tuple): EncryptionParams = match has_additional_params {
        CreatePoolParams::Neither => (None, None),
        CreatePoolParams::KeyDesc => (Some(get_next_arg(&mut iter, 3)?), None),
        CreatePoolParams::Both => (
            Some(get_next_arg(&mut iter, 3)?),
            Some(get_next_arg(&mut iter, 4)?),
        ),
    };

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

    let object_path = m.path.get_name();
    let dbus_context = m.tree.get_data();
    let mut lock = lock!(dbus_context.engine, write);
    let result = log_action!(lock.create_pool(
        name,
        &devs.map(|x| Path::new(x)).collect::<Vec<&Path>>(),
        tuple_to_option(redundancy_tuple),
        &EncryptionInfo {
            key_description: key_desc,
            clevis_info,
        }
    ));

    let msg = match result {
        Ok(pool_uuid_action) => {
            let results = match pool_uuid_action {
                CreateAction::Created(uuid) => {
                    let (_, pool) = get_pool!(lock; uuid; default_return; return_message);

                    let pool_object_path: dbus::Path = create_dbus_pool(
                        dbus_context,
                        object_path.clone(),
                        &Name::new(name.to_string()),
                        uuid,
                        &*lock!(pool, read),
                    );

                    let bd_paths = lock!(pool, read)
                        .blockdevs()
                        .into_iter()
                        .map(|(uuid, tier, bd)| {
                            create_dbus_blockdev(
                                dbus_context,
                                pool_object_path.clone(),
                                uuid,
                                tier,
                                bd,
                            )
                        })
                        .collect::<Vec<_>>();
                    (true, (pool_object_path, bd_paths))
                }
                CreateAction::Identity => default_return,
            };
            return_message.append3(results, msg_code_ok(), msg_string_ok())
        }
        Err(x) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&x);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn list_keys(info: &MethodInfo<MTSync<TData>, TData>) -> Result<Vec<String>, String> {
    let dbus_context = info.tree.get_data();

    let lock = lock!(dbus_context.engine, read);
    lock.get_key_handler()
        .list()
        .map(|v| {
            v.into_iter()
                .map(|kd| kd.as_application_str().to_string())
                .collect()
        })
        .map_err(|e| e.to_string())
}

pub fn set_key_shared(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let key_desc_str: String = get_next_arg(&mut iter, 0)?;
    let key_fd: OwnedFd = get_next_arg(&mut iter, 1)?;

    let dbus_context = m.tree.get_data();
    let default_return = (false, false);
    let return_message = message.method_return();

    let msg = match log_action!(lock!(dbus_context.engine, write).get_key_handler_mut().set(
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
            return_message.append3(return_value, msg_code_ok(), msg_string_ok())
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn locked_pool_uuids(info: &MethodInfo<MTSync<TData>, TData>) -> Result<Vec<String>, String> {
    let dbus_context = info.tree.get_data();

    let lock = lock!(dbus_context.engine, read);
    Ok(lock
        .locked_pools()
        .into_iter()
        .map(|(u, _)| uuid_to_string!(u))
        .collect())
}

pub fn locked_pools(
    info: &MethodInfo<MTSync<TData>, TData>,
) -> Result<HashMap<String, String>, String> {
    let dbus_context = info.tree.get_data();

    let engine = lock!(dbus_context.engine, read);
    Ok(engine
        .locked_pools()
        .into_iter()
        .map(|(u, info)| {
            (
                uuid_to_string!(u),
                info.info
                    .key_description
                    .map(|kd| kd.as_application_str().to_string())
                    .unwrap_or_else(String::new),
            )
        })
        .collect())
}

pub fn unlock_pool_shared(
    m: &MethodInfo<MTSync<TData>, TData>,
    take_unlock_arg: bool,
) -> MethodResult {
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
            let e = StratisError::Engine(
                ErrorEnum::Invalid,
                format!("Malformed UUID passed to UnlockPool: {}", e),
            );
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };
    let unlock_method = if take_unlock_arg {
        let unlock_method_str: &str = get_next_arg(&mut iter, 1)?;
        match UnlockMethod::try_from(unlock_method_str) {
            Ok(um) => um,
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                return Ok(vec![return_message.append3(default_return, rc, rs)]);
            }
        }
    } else {
        UnlockMethod::Keyring
    };

    let msg = match log_action!(
        lock!(dbus_context.engine, write).unlock_pool(pool_uuid, unlock_method)
    ) {
        Ok(unlock_action) => match unlock_action.changed() {
            Some(vec) => {
                let str_uuids: Vec<_> = vec.into_iter().map(|u| uuid_to_string!(u)).collect();
                return_message.append3((true, str_uuids), msg_code_ok(), msg_string_ok())
            }
            None => return_message.append3(default_return, msg_code_ok(), msg_string_ok()),
        },
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

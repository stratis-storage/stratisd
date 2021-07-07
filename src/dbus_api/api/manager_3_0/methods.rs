// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::convert::TryFrom;

use dbus::Message;
use dbus_tree::{MTSync, MethodInfo, MethodResult};

use crate::{
    dbus_api::{
        api::shared::{create_pool_shared, set_key_shared, unlock_pool_shared},
        consts,
        types::{DbusErrorEnum, TData, OK_STRING},
        util::{engine_to_dbus_err_tuple, get_next_arg},
    },
    engine::{DeleteAction, KeyDescription, MappingDeleteAction, PoolUuid},
};

pub fn destroy_pool(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
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

    let msg = match log_action!(dbus_context.engine.blocking_lock().destroy_pool(pool_uuid)) {
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

pub fn configure_simulator(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    let message = m.msg;
    let mut iter = message.iter_init();

    let return_message = message.method_return();

    let denominator: u32 = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let mut mutex_lock = dbus_context.engine.blocking_lock();
    let result = mutex_lock.configure_simulator(denominator);

    let msg = match result {
        Ok(_) => return_message.append2(DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append2(rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn unset_key(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let key_desc_str: String = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let default_return = false;
    let return_message = message.method_return();

    let msg = match log_action!(dbus_context
        .engine
        .blocking_lock()
        .get_key_handler_mut()
        .unset(&match KeyDescription::try_from(key_desc_str) {
            Ok(kd) => kd,
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                return Ok(vec![return_message.append3(default_return, rc, rs)]);
            }
        })) {
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

pub fn set_key(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    set_key_shared(m)
}

pub fn unlock_pool(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    unlock_pool_shared(m, true)
}

pub fn engine_state_report(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;

    let return_message = message.method_return();
    let default_return = String::new();

    let dbus_context = m.tree.get_data();
    let lock = dbus_context.engine.blocking_lock();

    let msg = match serde_json::to_string(&lock.engine_state_report()) {
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

pub fn create_pool(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    create_pool_shared(m)
}

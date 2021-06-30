// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::convert::TryFrom;

use dbus::Message;
use dbus_tree::{MTSync, MethodInfo, MethodResult};

use crate::{
    dbus_api::{
        types::{DbusErrorEnum, TData, OK_STRING},
        util::{engine_to_dbus_err_tuple, get_next_arg},
    },
    engine::{CreateAction, DeleteAction, KeyDescription},
};

pub fn bind_keyring(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();
    let key_desc_str: String = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = false;

    let key_desc = match KeyDescription::try_from(key_desc_str) {
        Ok(kd) => kd,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };

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

    let mut mutex_lock = dbus_context.engine.blocking_lock();
    let (_, pool) = get_mut_pool!(mutex_lock; pool_uuid; default_return; return_message);

    let msg = match handle_action!(
        pool.bind_keyring(&key_desc),
        dbus_context,
        pool_path.get_name()
    ) {
        Ok(CreateAction::Identity) => {
            return_message.append3(false, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Ok(CreateAction::Created(_)) => {
            return_message.append3(true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn unbind_keyring(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = false;

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

    let mut mutex_lock = dbus_context.engine.blocking_lock();
    let (_, pool) = get_mut_pool!(mutex_lock; pool_uuid; default_return; return_message);

    let msg = match handle_action!(pool.unbind_keyring(), dbus_context, pool_path.get_name()) {
        Ok(DeleteAction::Identity) => {
            return_message.append3(false, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Ok(DeleteAction::Deleted(_)) => {
            return_message.append3(true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

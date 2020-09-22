// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::convert::TryFrom;

use dbus::{
    tree::{MTFn, MethodInfo, MethodResult},
    Message,
};

use crate::{
    dbus_api::{
        types::{DbusErrorEnum, TData},
        util::{engine_to_dbus_err_tuple, get_next_arg, msg_code_ok, msg_string_ok},
    },
    engine::{CreateAction, DeleteAction, KeyDescription},
};

pub fn bind_clevis(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();
    let key_desc: String = get_next_arg(&mut iter, 0)?;
    let tang_info: &str = get_next_arg(&mut iter, 1)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = false;

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let (_, pool) = get_mut_pool!(engine; pool_uuid; default_return; return_message);

    let key_desc_typed = match KeyDescription::try_from(key_desc) {
        Ok(kd) => kd,
        Err(e) => {
            let msg = return_message.append3(
                default_return,
                DbusErrorEnum::ERROR as u16,
                format!("Invalid key description provided: {}", e),
            );
            return Ok(vec![msg]);
        }
    };
    let msg = match pool.bind_clevis(&key_desc_typed, tang_info) {
        Ok(CreateAction::Identity) => return_message.append3(false, msg_code_ok(), msg_string_ok()),
        Ok(CreateAction::Created(_)) => {
            return_message.append3(true, msg_code_ok(), msg_string_ok())
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn unbind_clevis(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = false;

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let (_, pool) = get_mut_pool!(engine; pool_uuid; default_return; return_message);

    let msg = match pool.unbind_clevis() {
        Ok(DeleteAction::Identity) => return_message.append3(false, msg_code_ok(), msg_string_ok()),
        Ok(DeleteAction::Deleted(_)) => {
            return_message.append3(true, msg_code_ok(), msg_string_ok())
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

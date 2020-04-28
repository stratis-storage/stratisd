// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::os::unix::io::RawFd;

use dbus::{
    tree::{MTFn, MethodInfo, MethodResult},
    Message,
};

use crate::{
    dbus_api::{
        api::shared::create_pool_shared,
        types::TData,
        util::{engine_to_dbus_err_tuple, get_next_arg, msg_code_ok, msg_string_ok},
    },
    engine::{CreateAction, DeleteAction},
};

pub fn create_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    create_pool_shared(m, true)
}

pub fn add_key(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let key_desc: &str = get_next_arg(&mut iter, 0)?;
    let key_fd: RawFd = get_next_arg(&mut iter, 1)?;

    let dbus_context = m.tree.get_data();
    let default_return = (false, false);
    let return_message = message.method_return();

    let msg = match dbus_context
        .engine
        .borrow_mut()
        .get_key_handler_mut()
        .add(key_desc, key_fd)
    {
        Ok(idem_resp) => {
            let return_value = match idem_resp {
                CreateAction::Created(is_changed) => (true, is_changed),
                _ => (false, false),
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

pub fn delete_key(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let key_desc: &str = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let default_return = false;
    let return_message = message.method_return();

    let msg = match dbus_context
        .engine
        .borrow_mut()
        .get_key_handler_mut()
        .delete(key_desc)
    {
        Ok(idem_resp) => {
            let return_value = match idem_resp {
                DeleteAction::Deleted(()) => true,
                _ => false,
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

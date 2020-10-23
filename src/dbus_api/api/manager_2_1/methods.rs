// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::convert::TryFrom;

use dbus::{
    tree::{MTSync, MethodInfo, MethodResult},
    Message,
};

use crate::{
    dbus_api::{
        api::shared::{create_pool_shared, set_key_shared, unlock_pool_shared},
        types::TData,
        util::{engine_to_dbus_err_tuple, get_next_arg, msg_code_ok, msg_string_ok},
    },
    engine::{DeleteAction, KeyDescription},
};

pub fn create_pool(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    create_pool_shared(m, true)
}

pub fn set_key(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    set_key_shared(m)
}

pub fn unset_key(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let key_desc_str: String = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let default_return = false;
    let return_message = message.method_return();

    let msg = match (*mutex_lock!(dbus_context.engine))
        .get_key_handler_mut()
        .unset(&match KeyDescription::try_from(key_desc_str) {
            Ok(kd) => kd,
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                return Ok(vec![return_message.append3(default_return, rc, rs)]);
            }
        }) {
        Ok(idem_resp) => {
            let return_value = matches!(idem_resp, DeleteAction::Deleted(()));
            return_message.append3(return_value, msg_code_ok(), msg_string_ok())
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn unlock_pool(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    unlock_pool_shared(m, false)
}

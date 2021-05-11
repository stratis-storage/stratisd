// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::Message;
use dbus_tree::{MTSync, MethodInfo, MethodResult};

use crate::dbus_api::{
    api::shared::create_pool_shared,
    types::{CreatePoolParams, TData},
    util::{engine_to_dbus_err_tuple, msg_code_ok, msg_string_ok},
};

pub fn engine_state_report(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;

    let return_message = message.method_return();
    let default_return = String::new();

    let dbus_context = m.tree.get_data();
    let lock = dbus_context.engine.blocking_lock();

    let msg = match serde_json::to_string(&lock.engine_state_report()) {
        Ok(string) => return_message.append3(string, msg_code_ok(), msg_string_ok()),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e.into());
            return_message.append3(default_return, rc, rs)
        }
    };

    Ok(vec![msg])
}

pub fn create_pool(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    create_pool_shared(m, CreatePoolParams::Both)
}

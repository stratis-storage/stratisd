// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::Message;
use dbus_tree::{MTSync, MethodInfo, MethodResult};

use crate::{
    dbus_api::{
        types::{DbusErrorEnum, TData},
        util::{
            engine_to_dbus_err_tuple, get_next_arg, msg_code_ok, msg_string_ok, tuple_to_option,
        },
    },
    engine::{DevUuid, RenameAction},
};

pub fn set_user_info(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let new_id_spec: (bool, &str) = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = (false, uuid_to_string!(DevUuid::nil()));

    let blockdev_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let blockdev_data = get_data!(blockdev_path; default_return; return_message);

    let pool_path = get_parent!(m; blockdev_data; default_return; return_message);
    let pool_uuid = typed_uuid!(
        get_data!(pool_path; default_return; return_message).uuid;
        Pool;
        default_return;
        return_message
    );

    let lock = lock!(dbus_context.engine, read);
    let (pool_name, pool) = get_pool!(lock; pool_uuid; default_return; return_message);

    let blockdev_uuid = typed_uuid!(blockdev_data.uuid; Dev; default_return; return_message);
    let result = lock!(pool, write).set_blockdev_user_info(
        &pool_name,
        blockdev_uuid,
        tuple_to_option(new_id_spec),
    );
    let msg = match log_action!(result) {
        Ok(RenameAction::NoSource) => {
            let error_message = format!(
                "pool doesn't know about block device {}",
                blockdev_data.uuid
            );
            let (rc, rs) = (DbusErrorEnum::INTERNAL_ERROR as u16, error_message);
            return_message.append3(default_return, rc, rs)
        }
        Ok(RenameAction::Renamed(uuid)) => return_message.append3(
            (true, uuid_to_string!(uuid)),
            msg_code_ok(),
            msg_string_ok(),
        ),
        Ok(RenameAction::Identity) => {
            return_message.append3(default_return, msg_code_ok(), msg_string_ok())
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };

    Ok(vec![msg])
}

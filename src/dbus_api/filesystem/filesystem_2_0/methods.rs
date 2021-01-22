// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{
    tree::{MTSync, MethodInfo, MethodResult},
    Message,
};

use crate::{
    dbus_api::{
        types::{DbusErrorEnum, TData},
        util::{engine_to_dbus_err_tuple, get_next_arg, msg_code_ok, msg_string_ok},
    },
    engine::{FilesystemUuid, RenameAction},
};

pub fn rename_filesystem(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let new_name: &str = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = (false, uuid_to_string!(FilesystemUuid::nil()));

    let filesystem_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let filesystem_data = get_data!(filesystem_path; default_return; return_message);

    let pool_path = get_parent!(m; filesystem_data; default_return; return_message);
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let mut mutex_lock = mutex_lock!(dbus_context.engine);
    let (pool_name, pool) = get_mut_pool!(mutex_lock; pool_uuid; default_return; return_message);

    let msg = match pool.rename_filesystem(&pool_name, filesystem_data.uuid, new_name) {
        Ok(RenameAction::NoSource) => {
            let error_message = format!(
                "pool {} doesn't know about filesystem {}",
                pool_uuid, filesystem_data.uuid
            );
            let (rc, rs) = (DbusErrorEnum::INTERNAL_ERROR as u16, error_message);
            return_message.append3(default_return, rc, rs)
        }
        Ok(RenameAction::Identity) => {
            return_message.append3(default_return, msg_code_ok(), msg_string_ok())
        }
        Ok(RenameAction::Renamed(uuid)) => return_message.append3(
            (true, uuid_to_string!(uuid)),
            msg_code_ok(),
            msg_string_ok(),
        ),
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };

    Ok(vec![msg])
}

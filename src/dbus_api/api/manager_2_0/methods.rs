// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{
    tree::{MTFn, MethodInfo, MethodResult},
    Message,
};

use crate::{
    dbus_api::{
        api::shared::create_pool_shared,
        consts,
        types::TData,
        util::{engine_to_dbus_err_tuple, get_next_arg, msg_code_ok, msg_string_ok},
    },
    engine::{DeleteAction, PoolUuid},
};

pub fn create_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    create_pool_shared(m, false)
}

pub fn destroy_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
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
        .map(|d| d.uuid)
    {
        Some(uuid) => uuid,
        None => {
            return Ok(vec![return_message.append3(
                default_return,
                msg_code_ok(),
                msg_string_ok(),
            )]);
        }
    };

    let mut engine = dbus_context.engine.borrow_mut();

    let result = pool_op_logging!("preparing to destroy pool with uuid {}", pool_uuid.to_simple_ref(); "{}"; engine.destroy_pool(pool_uuid));

    let msg = match result {
        Ok(DeleteAction::Deleted(uuid)) => {
            dbus_context.actions.borrow_mut().push_remove(
                &pool_path,
                m.tree,
                consts::pool_interface_list(),
            );
            return_message.append3(
                (true, uuid_to_string!(uuid)),
                msg_code_ok(),
                msg_string_ok(),
            )
        }
        Ok(DeleteAction::Identity) => {
            return_message.append3(default_return, msg_code_ok(), msg_string_ok())
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn configure_simulator(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message = m.msg;
    let mut iter = message.iter_init();

    let denominator: u32 = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let result = dbus_context
        .engine
        .borrow_mut()
        .configure_simulator(denominator);

    let return_message = message.method_return();

    let msg = match result {
        Ok(_) => return_message.append2(msg_code_ok(), msg_string_ok()),
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append2(rc, rs)
        }
    };
    Ok(vec![msg])
}

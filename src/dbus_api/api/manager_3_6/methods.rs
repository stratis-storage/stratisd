// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::Message;
use dbus_tree::{MTSync, MethodInfo, MethodResult};
use futures::executor::block_on;

use crate::{
    dbus_api::{
        consts,
        types::{DbusErrorEnum, TData, OK_STRING},
        util::{engine_to_dbus_err_tuple, get_next_arg},
    },
    engine::{Name, PoolIdentifier, PoolUuid, StopAction, StratisUuid},
    stratis::StratisError,
};

pub fn stop_pool(m: &MethodInfo<'_, MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();
    let dbus_context = m.tree.get_data();
    let default_return = (false, String::new());
    let return_message = message.method_return();

    let id_str: &str = get_next_arg(&mut iter, 0)?;
    let pool_id = {
        let id_type_str: &str = get_next_arg(&mut iter, 1)?;
        match id_type_str {
            "uuid" => match PoolUuid::parse_str(id_str) {
                Ok(u) => PoolIdentifier::Uuid(u),
                Err(e) => {
                    let (rc, rs) = engine_to_dbus_err_tuple(&e);
                    return Ok(vec![return_message.append3(default_return, rc, rs)]);
                }
            },
            "name" => PoolIdentifier::Name(Name::new(id_str.to_string())),
            _ => {
                let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Msg(format!(
                    "ID type {id_type_str} not recognized"
                )));
                return Ok(vec![return_message.append3(default_return, rc, rs)]);
            }
        }
    };

    // If Some(_), send a locked pool property change signal only if the pool is
    // encrypted. If None, the pool may already be stopped or not exist at all.
    // Both of these cases are handled by stop_pool and the value we provide
    // for send_locked_signal does not matter as send_locked_signal is only
    // used when a pool is newly stopped which can only occur if the pool is found
    // here.
    let send_locked_signal = block_on(dbus_context.engine.get_pool(pool_id.clone()))
        .map(|g| {
            let (_, _, p) = g.as_tuple();
            p.is_encrypted()
        })
        .unwrap_or(false);

    let msg = match handle_action!(block_on(dbus_context.engine.stop_pool(pool_id, true))) {
        Ok(StopAction::Stopped(pool_uuid)) => {
            match m.tree.iter().find_map(|opath| {
                opath
                    .get_data()
                    .as_ref()
                    .and_then(|op_cxt| match op_cxt.uuid {
                        StratisUuid::Pool(u) => {
                            if u == pool_uuid {
                                Some(opath.get_name())
                            } else {
                                None
                            }
                        }
                        StratisUuid::Fs(_) => None,
                        StratisUuid::Dev(_) => None,
                    })
            }) {
                Some(pool_path) => {
                    dbus_context.push_remove(pool_path, consts::pool_interface_list());
                    if send_locked_signal {
                        dbus_context
                            .push_locked_pools(block_on(dbus_context.engine.locked_pools()));
                    }
                    dbus_context.push_stopped_pools(block_on(dbus_context.engine.stopped_pools()));
                }
                None => {
                    warn!("Could not find pool D-Bus path for the pool that was just stopped");
                }
            }
            return_message.append3(
                (true, uuid_to_string!(pool_uuid)),
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            )
        }
        Ok(StopAction::CleanedUp(pool_uuid)) => return_message.append3(
            (true, uuid_to_string!(pool_uuid)),
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Ok(StopAction::Identity) => return_message.append3(
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };

    Ok(vec![msg])
}

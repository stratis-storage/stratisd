// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;

use dbus::{arg::Array, Message};
use dbus_tree::{MTSync, MethodInfo, MethodResult};
use futures::executor::block_on;

use crate::{
    dbus_api::{
        blockdev::create_dbus_blockdev,
        pool::create_dbus_pool,
        types::{DbusErrorEnum, TData, OK_STRING},
        util::{engine_to_dbus_err_tuple, get_next_arg, tuple_to_option},
    },
    engine::{CreateAction, EncryptionInfo, Engine, KeyDescription, Pool, PoolIdentifier},
    stratis::StratisError,
};

type EncryptionParams = (Option<(bool, String)>, Option<(bool, (String, String))>);

pub fn create_pool<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> MethodResult
where
    E: 'static + Engine,
{
    let base_path = m.path.get_name();
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let name: &str = get_next_arg(&mut iter, 0)?;
    let devs: Array<'_, &str, _> = get_next_arg(&mut iter, 1)?;
    let (key_desc_tuple, clevis_tuple): EncryptionParams = (
        Some(get_next_arg(&mut iter, 2)?),
        Some(get_next_arg(&mut iter, 3)?),
    );

    let return_message = message.method_return();

    let default_return: (bool, (dbus::Path<'static>, Vec<dbus::Path<'static>>)) =
        (false, (dbus::Path::default(), Vec::new()));

    let key_desc = match key_desc_tuple.and_then(tuple_to_option) {
        Some(kds) => match KeyDescription::try_from(kds) {
            Ok(kd) => Some(kd),
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                return Ok(vec![return_message.append3(default_return, rc, rs)]);
            }
        },
        None => None,
    };

    let clevis_info = match clevis_tuple.and_then(tuple_to_option) {
        Some((pin, json_string)) => match serde_json::from_str(json_string.as_str()) {
            Ok(j) => Some((pin, j)),
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Serde(e));
                return Ok(vec![return_message.append3(default_return, rc, rs)]);
            }
        },
        None => None,
    };

    let dbus_context = m.tree.get_data();
    let create_result = handle_action!(block_on(dbus_context.engine.create_pool(
        name,
        &devs.map(Path::new).collect::<Vec<&Path>>(),
        EncryptionInfo::from_options((key_desc, clevis_info)).as_ref(),
    )));
    match create_result {
        Ok(pool_uuid_action) => match pool_uuid_action {
            CreateAction::Created(uuid) => {
                let guard = match block_on(dbus_context.engine.get_pool(PoolIdentifier::Uuid(uuid)))
                {
                    Some(g) => g,
                    None => {
                        let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Msg(
                            format!("Pool with UUID {uuid} was successfully started but appears to have been removed before it could be exposed on the D-Bus")
                        ));
                        return Ok(vec![return_message.append3(default_return, rc, rs)]);
                    }
                };

                let (pool_name, pool_uuid, pool) = guard.as_tuple();
                let pool_path =
                    create_dbus_pool(dbus_context, base_path.clone(), &pool_name, pool_uuid, pool);
                let mut bd_paths = Vec::new();
                for (bd_uuid, tier, bd) in pool.blockdevs() {
                    bd_paths.push(create_dbus_blockdev(
                        dbus_context,
                        pool_path.clone(),
                        bd_uuid,
                        tier,
                        bd,
                    ));
                }

                Ok(vec![return_message.append3(
                    (true, (pool_path, bd_paths)),
                    DbusErrorEnum::OK as u16,
                    OK_STRING.to_string(),
                )])
            }
            CreateAction::Identity => Ok(vec![return_message.append3(
                default_return,
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            )]),
        },
        Err(x) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&x);
            Ok(vec![return_message.append3(default_return, rc, rs)])
        }
    }
}

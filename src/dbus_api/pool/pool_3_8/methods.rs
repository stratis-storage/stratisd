// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::iter::once;

use dbus::Message;
use dbus_tree::{MTSync, MethodInfo, MethodResult};

use crate::{
    dbus_api::{
        types::{DbusErrorEnum, TData, OK_STRING},
        util::{engine_to_dbus_err_tuple, get_next_arg, tuple_to_option},
    },
    engine::{CreateAction, EncryptionInfo, KeyDescription, PoolEncryptionInfo},
    stratis::StratisError,
};

pub fn encrypt_pool(m: &MethodInfo<'_, MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let key_desc_tuple: (bool, String) = get_next_arg(&mut iter, 0)?;
    let clevis_tuple: (bool, (String, String)) = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return: (bool, Vec<String>) = (false, Vec::new());

    let key_desc = match tuple_to_option(key_desc_tuple) {
        Some(kds) => match KeyDescription::try_from(kds) {
            Ok(kd) => Some(kd),
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                return Ok(vec![return_message.append3(default_return, rc, rs)]);
            }
        },
        None => None,
    };
    let clevis_info = match tuple_to_option(clevis_tuple) {
        Some((pin, json_string)) => match serde_json::from_str(json_string.as_str()) {
            Ok(j) => Some((pin, j)),
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Serde(e));
                return Ok(vec![return_message.append3(default_return, rc, rs)]);
            }
        },
        None => None,
    };
    let encryption_info = match EncryptionInfo::from_options((key_desc, clevis_info)) {
        Some(enc) => enc,
        None => {
            let (rc, rs) = (
                DbusErrorEnum::ERROR as u16,
                "Either a key description or Clevis info is required for this method".to_string(),
            );
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

    let mut guard = get_mut_pool!(dbus_context.engine; pool_uuid; default_return; return_message);
    let (_, _, pool) = guard.as_mut_tuple();

    let result = handle_action!(
        pool.encrypt_pool(pool_uuid, &encryption_info),
        dbus_context,
        pool_path.get_name()
    );
    let msg = match result {
        Ok(CreateAction::Created(_)) => {
            let pool_enc_info = PoolEncryptionInfo::from(once(&encryption_info));
            dbus_context
                .push_pool_key_desc_change(pool_path.get_name(), Some(pool_enc_info.clone()));
            dbus_context.push_pool_clevis_info_change(pool_path.get_name(), Some(pool_enc_info));
            dbus_context.push_pool_encryption_status_change(pool_path.get_name(), true);
            return_message.append3(true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Ok(CreateAction::Identity) => {
            return_message.append3(false, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

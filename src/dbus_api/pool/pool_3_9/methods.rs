// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::Message;
use dbus_tree::{MTSync, MethodInfo, MethodResult};
use either::Either;
use serde_json::from_str;

use crate::{
    dbus_api::{
        types::{DbusErrorEnum, EncryptionInfos, TData, OK_STRING},
        util::{engine_to_dbus_err_tuple, get_next_arg, tuple_to_option},
    },
    engine::{CreateAction, InputEncryptionInfo, KeyDescription},
    stratis::StratisError,
};

pub fn encrypt_pool(m: &MethodInfo<'_, MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let (key_desc_array, clevis_array): EncryptionInfos<'_> =
        (get_next_arg(&mut iter, 0)?, get_next_arg(&mut iter, 1)?);

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = false;

    let key_descs =
        match key_desc_array
            .into_iter()
            .try_fold(Vec::new(), |mut vec, (ts_opt, kd_str)| {
                let token_slot = tuple_to_option(ts_opt);
                let kd = KeyDescription::try_from(kd_str.to_string())?;
                vec.push((token_slot, kd));
                Ok(vec)
            }) {
            Ok(kds) => kds,
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                return Ok(vec![return_message.append3(default_return, rc, rs)]);
            }
        };

    let clevis_infos =
        match clevis_array
            .into_iter()
            .try_fold(Vec::new(), |mut vec, (ts_opt, pin, json_str)| {
                let token_slot = tuple_to_option(ts_opt);
                let json = from_str(json_str)?;
                vec.push((token_slot, (pin.to_owned(), json)));
                Ok(vec)
            }) {
            Ok(cis) => cis,
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                return Ok(vec![return_message.append3(default_return, rc, rs)]);
            }
        };

    let ei = match InputEncryptionInfo::new(key_descs, clevis_infos) {
        Ok(Some(opt)) => opt,
        Ok(None) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Msg(
                "Need at least one unlock method to encrypt pool".to_string(),
            ));
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
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
    let (name, _, pool) = guard.as_mut_tuple();

    let result = handle_action!(
        pool.encrypt_pool(&name, pool_uuid, &ei),
        dbus_context,
        pool_path.get_name()
    );
    let msg = match result {
        Ok(CreateAction::Created(_)) => {
            let encryption_info = match pool.encryption_info().clone() {
                Some(Either::Left(ei)) => ei,
                Some(Either::Right(_)) => {
                    unreachable!("online reencryption disabled on metadata V1")
                }
                None => unreachable!("Must have succeeded"),
            };
            if encryption_info.all_key_descriptions().count() > 0 {
                dbus_context.push_pool_key_desc_change(
                    pool_path.get_name(),
                    Some(Either::Left((true, encryption_info.clone()))),
                );
            }
            if encryption_info.all_clevis_infos().count() > 0 {
                dbus_context.push_pool_clevis_info_change(
                    pool_path.get_name(),
                    Some(Either::Left((true, encryption_info.clone()))),
                );
            }
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

pub fn reencrypt_pool(m: &MethodInfo<'_, MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = false;

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
    let (name, _, pool) = guard.as_mut_tuple();

    let result = handle_action!(
        pool.start_reencrypt_pool()
            .and_then(|key_info| pool.do_reencrypt_pool(pool_uuid, key_info))
            .and_then(|_| pool.finish_reencrypt_pool(&name, pool_uuid)),
        dbus_context,
        pool_path.get_name()
    );
    let msg = match result {
        Ok(_) => {
            dbus_context.push_pool_last_reencrypt_timestamp(object_path, pool.last_reencrypt());
            return_message.append3(true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn decrypt_pool(m: &MethodInfo<'_, MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = false;

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
    let (name, uuid, pool) = guard.as_mut_tuple();

    let result = handle_action!(
        pool.decrypt_pool(&name, uuid),
        dbus_context,
        pool_path.get_name()
    );
    let msg = match result {
        Ok(_) => {
            dbus_context.push_pool_key_desc_change(pool_path.get_name(), None);
            dbus_context.push_pool_clevis_info_change(pool_path.get_name(), None);
            dbus_context.push_pool_encryption_status_change(pool_path.get_name(), false);
            dbus_context.push_pool_last_reencrypt_timestamp(object_path, pool.last_reencrypt());
            return_message.append3(true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

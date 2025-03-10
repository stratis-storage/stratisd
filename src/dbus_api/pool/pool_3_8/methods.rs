// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::Message;
use dbus_tree::{MTSync, MethodInfo, MethodResult};
use either::Either;
use serde_json::{from_str, Value};

use crate::{
    dbus_api::{
        types::{DbusErrorEnum, EncryptionInfos, TData, OK_STRING},
        util::{engine_to_dbus_err_tuple, get_next_arg, tuple_to_option},
    },
    engine::{
        CreateAction, DeleteAction, InputEncryptionInfo, KeyDescription, OptionalTokenSlotInput,
        RenameAction, StratSigblockVersion,
    },
    stratis::StratisError,
};

pub fn bind_clevis(m: &MethodInfo<'_, MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();
    let pin: String = get_next_arg(&mut iter, 0)?;
    let json_string: String = get_next_arg(&mut iter, 1)?;
    let token_slot_tuple: (bool, u32) = get_next_arg(&mut iter, 2)?;

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

    let mut pool = get_mut_pool!(dbus_context.engine; pool_uuid; default_return; return_message);
    let lowest_token_slot = pool
        .encryption_info()
        .and_then(|either| either.left())
        .as_ref()
        .and_then(|ei| ei.single_clevis_info())
        .map(|(token_slot, _)| token_slot);

    let json: Value = match serde_json::from_str(&json_string) {
        Ok(j) => j,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Serde(e));
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };

    let token_slot = match tuple_to_option(token_slot_tuple) {
        Some(t) => OptionalTokenSlotInput::Some(t),
        None => match pool.metadata_version() {
            StratSigblockVersion::V1 => OptionalTokenSlotInput::Legacy,
            StratSigblockVersion::V2 => OptionalTokenSlotInput::None,
        },
    };

    let msg = match handle_action!(
        pool.bind_clevis(token_slot, pin.as_str(), &json),
        dbus_context,
        pool_path.get_name()
    ) {
        Ok(CreateAction::Identity) => {
            return_message.append3(false, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Ok(CreateAction::Created(_)) => {
            dbus_context.push_pool_clevis_info_change(
                pool_path.get_name(),
                pool.encryption_info()
                    .map(|either| either.map_left(|ei| (lowest_token_slot.is_none(), ei))),
            );
            return_message.append3(true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn unbind_clevis(m: &MethodInfo<'_, MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;

    let mut iter = message.iter_init();
    let token_slot_tuple: (bool, u32) = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = false;

    let token_slot = tuple_to_option(token_slot_tuple);

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

    let mut pool = get_mut_pool!(dbus_context.engine; pool_uuid; default_return; return_message);
    let lowest_token_slot = pool
        .encryption_info()
        .and_then(|either| either.left())
        .as_ref()
        .and_then(|ei| ei.single_clevis_info())
        .map(|(token_slot, _)| token_slot);

    let msg = match handle_action!(
        pool.unbind_clevis(token_slot),
        dbus_context,
        pool_path.get_name()
    ) {
        Ok(DeleteAction::Identity) => {
            return_message.append3(false, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Ok(DeleteAction::Deleted(_)) => {
            dbus_context.push_pool_clevis_info_change(
                pool_path.get_name(),
                pool.encryption_info().map(|either| {
                    either.map_left(|ei| {
                        (token_slot.is_some() && token_slot == lowest_token_slot, ei)
                    })
                }),
            );
            return_message.append3(true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn bind_keyring(m: &MethodInfo<'_, MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();
    let key_desc_str: String = get_next_arg(&mut iter, 0)?;
    let token_slot_tuple: (bool, u32) = get_next_arg(&mut iter, 1)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = false;

    let key_desc = match KeyDescription::try_from(key_desc_str) {
        Ok(kd) => kd,
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

    let mut pool = get_mut_pool!(dbus_context.engine; pool_uuid; default_return; return_message);
    let token_slot = match tuple_to_option(token_slot_tuple) {
        Some(t) => OptionalTokenSlotInput::Some(t),
        None => match pool.metadata_version() {
            StratSigblockVersion::V1 => OptionalTokenSlotInput::Legacy,
            StratSigblockVersion::V2 => OptionalTokenSlotInput::None,
        },
    };
    let lowest_token_slot = pool
        .encryption_info()
        .and_then(|either| either.left())
        .as_ref()
        .and_then(|ei| ei.single_key_description())
        .map(|(token_slot, _)| token_slot);

    let msg = match handle_action!(
        pool.bind_keyring(token_slot, &key_desc),
        dbus_context,
        pool_path.get_name()
    ) {
        Ok(CreateAction::Identity) => {
            return_message.append3(false, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Ok(CreateAction::Created(_)) => {
            dbus_context.push_pool_key_desc_change(
                pool_path.get_name(),
                pool.encryption_info()
                    .map(|ei| ei.map_left(|e| (lowest_token_slot.is_none(), e))),
            );
            return_message.append3(true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn unbind_keyring(m: &MethodInfo<'_, MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;

    let mut iter = message.iter_init();
    let token_slot_tuple: (bool, u32) = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = false;

    let token_slot = tuple_to_option(token_slot_tuple);

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

    let mut pool = get_mut_pool!(dbus_context.engine; pool_uuid; default_return; return_message);
    let lowest_token_slot = pool
        .encryption_info()
        .and_then(|either| either.left())
        .as_ref()
        .and_then(|ei| ei.single_key_description())
        .map(|(token_slot, _)| token_slot);

    let msg = match handle_action!(
        pool.unbind_keyring(token_slot),
        dbus_context,
        pool_path.get_name()
    ) {
        Ok(DeleteAction::Identity) => {
            return_message.append3(false, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Ok(DeleteAction::Deleted(_)) => {
            dbus_context.push_pool_key_desc_change(
                pool_path.get_name(),
                pool.encryption_info().map(|either| {
                    either.map_left(|ei| {
                        (token_slot.is_some() && token_slot == lowest_token_slot, ei)
                    })
                }),
            );
            return_message.append3(true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn rebind_keyring(m: &MethodInfo<'_, MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();
    let key_desc_str: String = get_next_arg(&mut iter, 0)?;
    let token_slot_tuple: (bool, u32) = get_next_arg(&mut iter, 1)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = false;

    let key_desc = match KeyDescription::try_from(key_desc_str) {
        Ok(kd) => kd,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };

    let token_slot = tuple_to_option(token_slot_tuple);

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

    let mut pool = get_mut_pool!(dbus_context.engine; pool_uuid; default_return; return_message);
    let lowest_token_slot = pool
        .encryption_info()
        .and_then(|either| either.left())
        .as_ref()
        .and_then(|ei| ei.single_key_description())
        .map(|(token_slot, _)| token_slot);

    let msg = match handle_action!(
        pool.rebind_keyring(token_slot, &key_desc),
        dbus_context,
        pool_path.get_name()
    ) {
        Ok(RenameAction::Identity) => {
            return_message.append3(false, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Ok(RenameAction::Renamed(_)) => {
            dbus_context.push_pool_key_desc_change(
                pool_path.get_name(),
                pool.encryption_info().map(|either| {
                    either.map_left(|ei| {
                        (token_slot.is_some() && token_slot == lowest_token_slot, ei)
                    })
                }),
            );
            return_message.append3(true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Ok(RenameAction::NoSource) => {
            let error_message = format!(
                "pool with UUID {pool_uuid} is not currently bound to a keyring passphrase"
            );
            let (rc, rs) = (DbusErrorEnum::ERROR as u16, error_message);
            return_message.append3(default_return, rc, rs)
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

pub fn rebind_clevis(m: &MethodInfo<'_, MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;

    let mut iter = message.iter_init();
    let token_slot_tuple: (bool, u32) = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = false;

    let token_slot = tuple_to_option(token_slot_tuple);

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

    let mut pool = get_mut_pool!(dbus_context.engine; pool_uuid; default_return; return_message);
    let lowest_token_slot = pool
        .encryption_info()
        .and_then(|either| either.left())
        .as_ref()
        .and_then(|ei| ei.single_clevis_info())
        .map(|(token_slot, _)| token_slot);

    let msg = match handle_action!(
        pool.rebind_clevis(token_slot),
        dbus_context,
        pool_path.get_name()
    ) {
        Ok(_) => {
            dbus_context.push_pool_clevis_info_change(
                pool_path.get_name(),
                pool.encryption_info().map(|either| {
                    either.map_left(|ei| {
                        (token_slot.is_some() && token_slot == lowest_token_slot, ei)
                    })
                }),
            );
            return_message.append3(true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

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
    let (_, _, pool) = guard.as_mut_tuple();

    let result = handle_action!(pool.reencrypt_pool(), dbus_context, pool_path.get_name());
    let msg = match result {
        Ok(_) => {
            dbus_context.push_pool_clevis_info_change(pool_path.get_name(), None);
            dbus_context.push_pool_encryption_status_change(pool_path.get_name(), false);
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
        Ok(_) => return_message.append3(true, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

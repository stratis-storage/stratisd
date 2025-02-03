// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use serde_json::from_str;
use tokio::sync::RwLock;
use zbus::Connection;

use crate::{
    dbus::{
        consts::OK_STRING,
        manager::Manager,
        types::DbusErrorEnum,
        util::{
            engine_to_dbus_err_tuple, send_clevis_info_signal, send_encrypted_signal,
            send_keyring_signal, tuple_to_option,
        },
    },
    engine::{
        CreateAction, DeleteAction, Engine, InputEncryptionInfo, KeyDescription, Lockable,
        PoolIdentifier, PoolUuid,
    },
    stratis::StratisError,
};

pub async fn encrypt_pool_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
    key_descs: Vec<((bool, u32), KeyDescription)>,
    clevis_infos: Vec<((bool, u32), &str, &str)>,
) -> (bool, u16, String) {
    let default_return = false;

    let key_descs_parsed =
        match key_descs
            .into_iter()
            .try_fold(Vec::new(), |mut vec, (ts_opt, kd)| {
                let token_slot = tuple_to_option(ts_opt);
                vec.push((token_slot, kd));
                Ok(vec)
            }) {
            Ok(kds) => kds,
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                return (default_return, rc, rs);
            }
        };

    let clevis_infos_parsed =
        match clevis_infos
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
                return (default_return, rc, rs);
            }
        };

    let iei = match InputEncryptionInfo::new(key_descs_parsed, clevis_infos_parsed) {
        Ok(Some(info)) => info,
        Ok(None) => {
            return (
                default_return,
                DbusErrorEnum::ERROR as u16,
                "No unlock methods provided".to_string(),
            );
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return (default_return, rc, rs);
        }
    };

    let guard_res = engine
        .get_mut_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;

        let (name, _, pool) = guard.as_mut_tuple();

        handle_action!(pool.encrypt_pool(&name, pool_uuid, &iei))
    })
    .await
    {
        Ok(Ok(CreateAction::Created(_))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_keyring_signal(connection, &p.as_ref(), true).await;
                    send_clevis_info_signal(connection, &p.as_ref(), true).await;
                    send_encrypted_signal(connection, &p.as_ref()).await;
                }
                None => {
                    warn!("No pool path associated with UUID {pool_uuid}; failed to send encryption related signals");
                }
            }
            (true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Ok(Ok(CreateAction::Identity)) => (false, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Ok(Err(e)) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            (default_return, rc, rs)
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::from(e));
            (default_return, rc, rs)
        }
    }
}

pub async fn reencrypt_pool_method(
    engine: &Arc<dyn Engine>,
    pool_uuid: PoolUuid,
) -> (bool, u16, String) {
    let default_return = false;

    let guard_res = engine
        .get_mut_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;

        handle_action!(guard.reencrypt_pool(pool_uuid))
    })
    .await
    {
        Ok(Ok(_)) => (true, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Ok(Err(e)) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            (default_return, rc, rs)
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::from(e));
            (default_return, rc, rs)
        }
    }
}

pub async fn decrypt_pool_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
) -> (bool, u16, String) {
    let default_return = false;

    let guard_res = engine
        .get_mut_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;

        let (name, _, pool) = guard.as_mut_tuple();

        handle_action!(pool.decrypt_pool(&name, pool_uuid))
    })
    .await
    {
        Ok(Ok(DeleteAction::Deleted(_))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_keyring_signal(connection, &p.as_ref(), true).await;
                    send_clevis_info_signal(connection, &p.as_ref(), true).await;
                    send_encrypted_signal(connection, &p.as_ref()).await;
                }
                None => {
                    warn!("No pool path associated with UUID {pool_uuid}; failed to send encryption related signals");
                }
            }
            (true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Ok(Ok(DeleteAction::Identity)) => (false, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Ok(Err(e)) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            (default_return, rc, rs)
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::from(e));
            (default_return, rc, rs)
        }
    }
}

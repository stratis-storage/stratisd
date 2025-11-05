// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use serde_json::from_str;
use tokio::task::spawn_blocking;
use zbus::Connection;

use crate::{
    dbus::{
        consts::OK_STRING,
        types::DbusErrorEnum,
        util::{engine_to_dbus_err_tuple, tuple_to_option},
    },
    engine::{
        CreateAction, EncryptedDevice, Engine, InputEncryptionInfo, KeyDescription, PoolIdentifier,
        PoolUuid,
    },
    stratis::StratisError,
};

#[allow(clippy::too_many_arguments)]
pub async fn encrypt_pool_method(
    _connection: &Arc<Connection>,
    engine: &Arc<dyn Engine>,
    pool_uuid: PoolUuid,
    key_desc_array: Vec<((bool, u32), &str)>,
    clevis_array: Vec<((bool, u32), &str, &str)>,
) -> (bool, u16, String) {
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
                return (default_return, rc, rs);
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
                return (default_return, rc, rs);
            }
        };

    let ei = match InputEncryptionInfo::new(key_descs, clevis_infos) {
        Ok(Some(opt)) => opt,
        Ok(None) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Msg(
                "Need at least one unlock method to encrypt pool".to_string(),
            ));
            return (default_return, rc, rs);
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return (default_return, rc, rs);
        }
    };

    let mut guard = match engine
        .get_mut_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")))
    {
        Ok(g) => g,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return (default_return, rc, rs);
        }
    };

    let create_result = match spawn_blocking(move || {
        guard
            .start_encrypt_pool(pool_uuid, &ei)
            .map(|action| (guard, action))
    })
    .await
    {
        Ok(Ok((_, CreateAction::Identity))) => Ok(CreateAction::Identity),
        Ok(Ok((guard, CreateAction::Created((sector_size, key_info))))) => {
            let guard = guard.downgrade();
            match spawn_blocking(move || {
                guard
                    .do_encrypt_pool(pool_uuid, sector_size, key_info)
                    .map(|_| guard)
            })
            .await
            {
                Ok(Ok(guard)) => {
                    let mut guard = engine.upgrade_pool(guard).await;
                    let (name, _, _) = guard.as_mut_tuple();
                    match spawn_blocking(move || guard.finish_encrypt_pool(&name, pool_uuid)).await
                    {
                        Ok(Ok(_)) => Ok(CreateAction::Created(EncryptedDevice(pool_uuid))),
                        Ok(Err(e)) => Err(e),
                        Err(e) => Err(StratisError::from(e)),
                    }
                }
                Ok(Err(e)) => Err(e),
                Err(e) => Err(StratisError::from(e)),
            }
        }
        Ok(Err(e)) => Err(e),
        Err(e) => Err(StratisError::from(e)),
    };

    match create_result {
        Ok(CreateAction::Identity) => (
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Ok(CreateAction::Created(_)) => (true, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            (default_return, rc, rs)
        }
    }
}

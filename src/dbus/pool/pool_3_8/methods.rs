// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use tokio::sync::RwLock;
use zbus::Connection;

use crate::{
    dbus::{
        consts::OK_STRING,
        manager::Manager,
        types::DbusErrorEnum,
        util::{
            engine_to_dbus_err_tuple, send_clevis_info_signal, send_free_token_slots_signal,
            send_keyring_signal, tuple_to_option,
        },
    },
    engine::{
        CreateAction, DeleteAction, Engine, KeyDescription, Lockable, OptionalTokenSlotInput,
        PoolIdentifier, PoolUuid, RenameAction, StratSigblockVersion,
    },
    stratis::StratisError,
};

pub async fn bind_clevis_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
    pin: String,
    json: &str,
    token_slot_tuple: (bool, u32),
) -> (bool, u16, String) {
    let default_return = false;

    let json_value = match serde_json::from_str::<serde_json::Value>(json) {
        Ok(j) => j,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::from(e));
            return (default_return, rc, rs);
        }
    };

    let guard_res = engine
        .get_mut_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    let conn_clone = Arc::clone(connection);
    let man_clone = manager.clone();
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;

        let (name, _, pool) = guard.as_mut_tuple();

        let token_slot = match tuple_to_option(token_slot_tuple) {
            Some(t) => OptionalTokenSlotInput::Some(t),
            None => match pool.metadata_version() {
                StratSigblockVersion::V1 => OptionalTokenSlotInput::Legacy,
                StratSigblockVersion::V2 => OptionalTokenSlotInput::None,
            },
        };
        let free_token_slots = pool.free_token_slots();
        let lowest_token_slot = pool
            .encryption_info()
            .and_then(|either| either.left())
            .as_ref()
            .and_then(|ei| ei.single_clevis_info())
            .map(|(token_slot, _)| token_slot);
        let action = handle_action!(
            pool.bind_clevis(&name, token_slot, pin.as_str(), &json_value,),
            conn_clone,
            man_clone,
            pool_uuid
        );
        let new_free_token_slots = pool.free_token_slots();
        match action {
            Ok(CreateAction::Created(_)) => Ok(CreateAction::Created((
                free_token_slots,
                new_free_token_slots,
                lowest_token_slot,
                pool.encryption_info()
                    .map(|e| e.is_right())
                    .unwrap_or(false),
            ))),
            Ok(CreateAction::Identity) => Ok(CreateAction::Identity),
            Err(e) => Err(e),
        }
    })
    .await
    {
        Ok(Ok(CreateAction::Identity)) => (false, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Ok(Ok(CreateAction::Created((fts, nfts, low_ts, is_right)))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_clevis_info_signal(connection, p, low_ts.is_none() || is_right).await;
                    if fts != nfts {
                        send_free_token_slots_signal(connection, p).await;
                    }
                }
                None => {
                    warn!("No object path associated with pool UUID {pool_uuid}; failed to send pool free token slots change signals");
                }
            };
            (true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
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

pub async fn bind_keyring_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
    kd: KeyDescription,
    token_slot_tuple: (bool, u32),
) -> (bool, u16, String) {
    let default_return = false;

    let guard_res = engine
        .get_mut_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    let conn_clone = Arc::clone(connection);
    let man_clone = manager.clone();
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;

        let (name, _, pool) = guard.as_mut_tuple();

        let token_slot = match tuple_to_option(token_slot_tuple) {
            Some(t) => OptionalTokenSlotInput::Some(t),
            None => match pool.metadata_version() {
                StratSigblockVersion::V1 => OptionalTokenSlotInput::Legacy,
                StratSigblockVersion::V2 => OptionalTokenSlotInput::None,
            },
        };
        let free_token_slots = pool.free_token_slots();
        let lowest_token_slot = pool
            .encryption_info()
            .and_then(|either| either.left())
            .as_ref()
            .and_then(|ei| ei.single_clevis_info())
            .map(|(token_slot, _)| token_slot);

        let action = handle_action!(
            pool.bind_keyring(&name, token_slot, &kd),
            conn_clone,
            man_clone,
            pool_uuid
        );
        let new_free_token_slots = pool.free_token_slots();
        match action {
            Ok(CreateAction::Created(_)) => Ok(CreateAction::Created((
                free_token_slots,
                new_free_token_slots,
                lowest_token_slot,
                pool.encryption_info()
                    .map(|e| e.is_right())
                    .unwrap_or(false),
            ))),
            Ok(CreateAction::Identity) => Ok(CreateAction::Identity),
            Err(e) => Err(e),
        }
    })
    .await
    {
        Ok(Ok(CreateAction::Identity)) => (false, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Ok(Ok(CreateAction::Created((fts, nfts, low_ts, is_right)))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_keyring_signal(connection, p, low_ts.is_none() || is_right).await;
                    if fts != nfts {
                        send_free_token_slots_signal(connection, p).await;
                    }
                }
                None => {
                    warn!("No object path associated with pool UUID {pool_uuid}; failed to send pool free token slots change signals");
                }
            };
            (true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
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

pub async fn rebind_clevis_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
    token_slot_tuple: (bool, u32),
) -> (bool, u16, String) {
    let default_return = false;

    let token_slot = tuple_to_option(token_slot_tuple);

    let guard_res = engine
        .get_mut_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));

    let conn_clone = Arc::clone(connection);
    let man_clone = manager.clone();
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;

        let lowest_token_slot = guard
            .encryption_info()
            .and_then(|either| either.left())
            .as_ref()
            .and_then(|ei| ei.single_clevis_info())
            .map(|(token_slot, _)| token_slot);

        handle_action!(
            guard.rebind_clevis(token_slot),
            conn_clone,
            man_clone,
            pool_uuid
        )
        .map(|_| {
            (
                guard
                    .encryption_info()
                    .map(|e| e.is_right())
                    .unwrap_or(false),
                lowest_token_slot,
            )
        })
    })
    .await
    {
        Ok(Ok((is_right, low_ts))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_clevis_info_signal(
                        connection,
                        p,
                        token_slot.is_none()
                            || (token_slot.is_some() && low_ts == token_slot)
                            || is_right,
                    )
                    .await
                }
                None => {
                    warn!("Failed to find pool path for pool with UUID {pool_uuid}");
                }
            }
            (true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
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

pub async fn rebind_keyring_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
    key_desc: KeyDescription,
    token_slot_tuple: (bool, u32),
) -> (bool, u16, String) {
    let default_return = false;

    let token_slot = tuple_to_option(token_slot_tuple);

    let guard_res = engine
        .get_mut_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    let conn_clone = Arc::clone(connection);
    let man_clone = manager.clone();
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;

        let lowest_token_slot = guard
            .encryption_info()
            .and_then(|either| either.left())
            .as_ref()
            .and_then(|ei| ei.single_clevis_info())
            .map(|(token_slot, _)| token_slot);
        handle_action!(
            guard.rebind_keyring(token_slot, &key_desc),
            conn_clone,
            man_clone,
            pool_uuid
        )
        .map(|action| match action {
            RenameAction::Renamed(_) => RenameAction::Renamed((
                guard
                    .encryption_info()
                    .map(|e| e.is_right())
                    .unwrap_or(false),
                lowest_token_slot,
            )),
            RenameAction::Identity => RenameAction::Identity,
            RenameAction::NoSource => RenameAction::NoSource,
        })
    })
    .await
    {
        Ok(Ok(RenameAction::Renamed((is_right, low_ts)))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_keyring_signal(
                        connection,
                        p,
                        token_slot.is_none()
                            || (token_slot.is_some() && token_slot == low_ts)
                            || is_right,
                    )
                    .await
                }
                None => {
                    warn!("Failed to find pool path for pool with UUID {pool_uuid}");
                }
            }
            (true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Ok(Ok(RenameAction::Identity)) => (false, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Ok(Ok(RenameAction::NoSource)) => (
            false,
            DbusErrorEnum::ERROR as u16,
            format!("pool with UUID {pool_uuid} is not currently bound to a keyring passphrase"),
        ),
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

pub async fn unbind_clevis_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
    token_slot_tuple: (bool, u32),
) -> (bool, u16, String) {
    let default_return = false;

    let token_slot = tuple_to_option(token_slot_tuple);

    let guard_res = engine
        .get_mut_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    let conn_clone = Arc::clone(connection);
    let man_clone = manager.clone();
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;
        let (name, _, pool) = guard.as_mut_tuple();

        let lowest_token_slot = pool
            .encryption_info()
            .and_then(|either| either.left())
            .as_ref()
            .and_then(|ei| ei.single_clevis_info())
            .map(|(token_slot, _)| token_slot);

        handle_action!(
            pool.unbind_clevis(&name, token_slot),
            conn_clone,
            man_clone,
            pool_uuid
        )
        .map(|action| match action {
            DeleteAction::Deleted(_) => DeleteAction::Deleted((
                guard
                    .encryption_info()
                    .map(|e| e.is_right())
                    .unwrap_or(false),
                lowest_token_slot,
            )),
            DeleteAction::Identity => DeleteAction::Identity,
        })
    })
    .await
    {
        Ok(Ok(DeleteAction::Deleted((is_right, low_ts)))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_clevis_info_signal(
                        connection,
                        p,
                        token_slot.is_none()
                            || (token_slot.is_some() && token_slot == low_ts)
                            || is_right,
                    )
                    .await;
                    send_free_token_slots_signal(connection, p).await;
                }
                None => {
                    warn!("Failed to find pool path for pool with UUID {pool_uuid}");
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

pub async fn unbind_keyring_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
    token_slot_tuple: (bool, u32),
) -> (bool, u16, String) {
    let default_return = false;

    let token_slot = tuple_to_option(token_slot_tuple);

    let guard_res = engine
        .get_mut_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    let conn_clone = Arc::clone(connection);
    let man_clone = manager.clone();
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;
        let (name, _, pool) = guard.as_mut_tuple();

        let lowest_token_slot = pool
            .encryption_info()
            .and_then(|either| either.left())
            .as_ref()
            .and_then(|ei| ei.single_clevis_info())
            .map(|(token_slot, _)| token_slot);

        handle_action!(
            pool.unbind_keyring(&name, token_slot),
            conn_clone,
            man_clone,
            pool_uuid
        )
        .map(|action| match action {
            DeleteAction::Deleted(_) => DeleteAction::Deleted((
                guard
                    .encryption_info()
                    .map(|e| e.is_right())
                    .unwrap_or(false),
                lowest_token_slot,
            )),
            DeleteAction::Identity => DeleteAction::Identity,
        })
    })
    .await
    {
        Ok(Ok(DeleteAction::Deleted((is_right, low_ts)))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_keyring_signal(
                        connection,
                        p,
                        token_slot.is_none()
                            || (token_slot.is_some() && token_slot == low_ts)
                            || is_right,
                    )
                    .await;
                    send_free_token_slots_signal(connection, p).await;
                }
                None => {
                    warn!("Failed to find pool path for pool with UUID {pool_uuid}");
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

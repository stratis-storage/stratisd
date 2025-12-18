// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    os::fd::AsRawFd,
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc},
};

use serde_json::from_str;
use tokio::sync::RwLock;
use zbus::{
    zvariant::{Fd, ObjectPath, OwnedObjectPath},
    Connection,
};

use crate::{
    dbus::{
        consts::OK_STRING,
        manager::Manager,
        pool::{register_pool, unregister_pool},
        types::DbusErrorEnum,
        util::{engine_to_dbus_err_tuple, send_locked_pools_signals, tuple_to_option},
    },
    engine::{
        CreateAction, DeleteAction, DevUuid, Engine, InputEncryptionInfo, IntegritySpec,
        KeyDescription, Lockable, MappingCreateAction, MappingDeleteAction, PoolUuid,
        SetUnlockAction, UnlockMethod,
    },
    stratis::StratisError,
};

pub async fn list_keys_method(engine: &Arc<dyn Engine>) -> (Vec<KeyDescription>, u16, String) {
    let default_return = Vec::new();

    match engine.get_key_handler().await.list() {
        Ok(vec) => (vec, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            (default_return, rc, rs)
        }
    }
}

pub async fn set_key_method(
    engine: &Arc<dyn Engine>,
    key_desc: &KeyDescription,
    fd: Fd<'_>,
) -> ((bool, bool), u16, String) {
    let default_return = (false, false);

    match handle_action!(engine.get_key_handler().await.set(key_desc, fd.as_raw_fd())) {
        Ok(MappingCreateAction::Created(_)) => (
            (true, false),
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Ok(MappingCreateAction::ValueChanged(_)) => (
            (true, true),
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Ok(MappingCreateAction::Identity) => (
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            (default_return, rc, rs)
        }
    }
}

pub async fn unset_key_method(
    engine: &Arc<dyn Engine>,
    key_desc: &KeyDescription,
) -> (bool, u16, String) {
    let default_return = false;

    match handle_action!(engine.get_key_handler().await.unset(key_desc)) {
        Ok(MappingDeleteAction::Deleted(_)) => {
            (true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Ok(MappingDeleteAction::Identity) => (
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            (default_return, rc, rs)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn create_pool_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    name: &str,
    devs: Vec<PathBuf>,
    key_desc: (bool, KeyDescription),
    clevis_info: (bool, (&str, &str)),
) -> ((bool, (OwnedObjectPath, Vec<OwnedObjectPath>)), u16, String) {
    let default_return = (false, (OwnedObjectPath::default(), Vec::new()));

    let devs_ref = devs.iter().map(|path| path.as_path()).collect::<Vec<_>>();
    let key_desc = tuple_to_option(key_desc);
    let clevis_info = match tuple_to_option(clevis_info) {
        Some((pin, json_string)) => match from_str(json_string) {
            Ok(j) => Some((pin.to_owned(), j)),
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::from(e));
                return (default_return, rc, rs);
            }
        },
        None => None,
    };
    let iei = InputEncryptionInfo::new_legacy(key_desc, clevis_info);

    match engine
        .create_pool(
            name,
            devs_ref.as_slice(),
            iei.as_ref(),
            IntegritySpec::default(),
        )
        .await
    {
        Ok(CreateAction::Created(uuid)) => {
            match register_pool(engine, connection, manager, counter, uuid).await {
                Ok((pool_path, fs_paths)) => (
                    (
                        true,
                        (
                            OwnedObjectPath::from(pool_path),
                            fs_paths.into_iter().map(OwnedObjectPath::from).collect(),
                        ),
                    ),
                    DbusErrorEnum::OK as u16,
                    OK_STRING.to_string(),
                ),
                Err(e) => {
                    let (rc, rs) = engine_to_dbus_err_tuple(&e);
                    (default_return, rc, rs)
                }
            }
        }
        Ok(CreateAction::Identity) => (
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            (default_return, rc, rs)
        }
    }
}

pub async fn destroy_pool_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool: ObjectPath<'_>,
) -> ((bool, String), u16, String) {
    let default_return = (false, String::default());

    let uuid = {
        let manager_lock = manager.write().await;
        match manager_lock.pool_get_uuid(&pool) {
            Some(u) => u,
            None => {
                return (
                    default_return,
                    DbusErrorEnum::ERROR as u16,
                    format!("Object path {pool} not associated with pool"),
                );
            }
        }
    };

    match engine.destroy_pool(uuid).await {
        Ok(DeleteAction::Deleted(uuid)) => {
            match unregister_pool(connection, manager, &pool).await {
                Ok(u) => {
                    assert_eq!(uuid, u);
                    (
                        (true, u.to_string()),
                        DbusErrorEnum::OK as u16,
                        OK_STRING.to_string(),
                    )
                }
                Err(e) => {
                    let (rc, rs) = engine_to_dbus_err_tuple(&e);
                    (default_return, rc, rs)
                }
            }
        }
        Ok(DeleteAction::Identity) => (
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            (default_return, rc, rs)
        }
    }
}

pub async fn unlock_pool_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    pool_uuid: PoolUuid,
    unlock_method: UnlockMethod,
) -> ((bool, Vec<DevUuid>), u16, String) {
    let default_return = (false, Vec::default());

    match handle_action!(engine.unlock_pool(pool_uuid, unlock_method).await) {
        Ok(SetUnlockAction::Started(v)) => {
            send_locked_pools_signals(connection).await;

            ((true, v), DbusErrorEnum::OK as u16, OK_STRING.to_string())
        }
        Ok(SetUnlockAction::Identity) => (
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            (default_return, rc, rs)
        }
    }
}

pub fn engine_state_report_method(engine: &Arc<dyn Engine>) -> (String, u16, String) {
    match serde_json::to_string(&engine.engine_state_report()) {
        Ok(result) => (result, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e.into());
            (String::new(), rc, rs)
        }
    }
}

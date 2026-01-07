// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc},
};

use tokio::sync::RwLock;
use zbus::{
    zvariant::{ObjectPath, OwnedObjectPath},
    Connection,
};

use devicemapper::Bytes;

use crate::{
    dbus::{
        blockdev::register_blockdev,
        consts::OK_STRING,
        filesystem::{register_filesystem, unregister_filesystem},
        manager::Manager,
        types::DbusErrorEnum,
        util::{
            engine_to_dbus_err_tuple, send_clevis_info_signal, send_free_token_slots_signal,
            send_has_cache_signal, send_keyring_signal, send_pool_foreground_signals,
            send_pool_name_signal, tuple_to_option,
        },
    },
    engine::{
        BlockDevTier, CreateAction, DeleteAction, Engine, EngineAction, KeyDescription, Lockable,
        OptionalTokenSlotInput, PoolIdentifier, PoolUuid, RenameAction,
    },
    stratis::StratisError,
};

pub async fn create_filesystems_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    pool_uuid: PoolUuid,
    filesystems: Vec<(&str, (bool, &str))>,
) -> ((bool, Vec<OwnedObjectPath>), u16, String) {
    let default_return = (false, (Vec::new()));

    let filesystem_specs = match filesystems
        .into_iter()
        .map(|(name, size_opt)| {
            let size = tuple_to_option(size_opt)
                .map(|val| {
                    val.parse::<u128>().map_err(|_| {
                        format!("Could not parse filesystem size string {val} to integer value")
                    })
                })
                .transpose()?;
            Ok((name.to_string(), size.map(Bytes), None))
        })
        .collect::<Result<Vec<(String, Option<Bytes>, Option<Bytes>)>, String>>()
    {
        Ok(fs_specs) => fs_specs,
        Err(e) => {
            let (rc, rs) = (DbusErrorEnum::ERROR as u16, e);
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
        handle_action!(
            pool.create_filesystems(
                name.to_string().as_str(),
                pool_uuid,
                filesystem_specs
                    .iter()
                    .map(|(s, b1, b2)| (s.as_str(), *b1, *b2))
                    .collect::<Vec<_>>()
                    .as_slice(),
            ),
            conn_clone,
            man_clone,
            pool_uuid
        )
    })
    .await
    {
        Ok(Ok(changed)) => {
            let mut object_paths = Vec::new();
            match changed.changed() {
                Some(v) => {
                    for (_, uuid, _) in v {
                        match register_filesystem(
                            engine, connection, manager, counter, pool_uuid, uuid,
                        )
                        .await
                        {
                            Ok(path) => {
                                object_paths.push(OwnedObjectPath::from(path));
                            }
                            Err(e) => {
                                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                                return (default_return, rc, rs);
                            }
                        }
                    }
                    (
                        (true, object_paths),
                        DbusErrorEnum::OK as u16,
                        OK_STRING.to_string(),
                    )
                }
                None => (
                    default_return,
                    DbusErrorEnum::OK as u16,
                    OK_STRING.to_string(),
                ),
            }
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

pub async fn destroy_filesystems_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
    filesystems: Vec<ObjectPath<'_>>,
) -> ((bool, Vec<String>), u16, String) {
    let default_return = (false, (Vec::new()));

    let uuids = {
        let lock = manager.read().await;
        filesystems
            .iter()
            .filter_map(|op| lock.filesystem_get_uuid(op))
            .collect::<HashSet<_>>()
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
        handle_action!(
            pool.destroy_filesystems(name.to_string().as_str(), &uuids),
            conn_clone,
            man_clone,
            pool_uuid
        )
    })
    .await
    {
        Ok(Ok(changed)) => match changed.changed() {
            Some((v, _)) => {
                for uuid in v.iter() {
                    let opt = manager.read().await.filesystem_get_path(uuid).cloned();
                    match opt {
                        Some(p) => {
                            if let Err(e) =
                                unregister_filesystem(connection, manager, &p.as_ref()).await
                            {
                                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                                return (default_return, rc, rs);
                            }
                            // TODO: Add signal handling for origin updating.
                        }
                        None => {
                            warn!("No path found to unregister for destroyed filesystem with UUID {uuid}");
                        }
                    }
                }
                (
                    (
                        true,
                        v.into_iter().map(|u| u.simple().to_string()).collect(),
                    ),
                    DbusErrorEnum::OK as u16,
                    OK_STRING.to_string(),
                )
            }
            None => (
                default_return,
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            ),
        },
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

pub async fn snapshot_filesystem_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    pool_uuid: PoolUuid,
    filesystem: ObjectPath<'_>,
    snapshot_name: String,
) -> ((bool, OwnedObjectPath), u16, String) {
    let default_return = (false, OwnedObjectPath::default());

    let fs_uuid = match manager.read().await.filesystem_get_uuid(&filesystem) {
        Some(u) => u,
        None => {
            return (
                default_return,
                DbusErrorEnum::ERROR as u16,
                format!("No filesystem UUID associated with path {filesystem}"),
            );
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
        handle_action!(
            pool.snapshot_filesystem(
                name.to_string().as_str(),
                pool_uuid,
                fs_uuid,
                snapshot_name.as_str()
            ),
            conn_clone,
            man_clone,
            pool_uuid
        )
        .map(|act| match act {
            CreateAction::Created((uuid, _)) => CreateAction::Created(uuid),
            CreateAction::Identity => CreateAction::Identity,
        })
    })
    .await
    {
        Ok(Ok(CreateAction::Created(snapshot_uuid))) => {
            let path = match register_filesystem(
                engine,
                connection,
                manager,
                counter,
                pool_uuid,
                snapshot_uuid,
            )
            .await
            {
                Ok(p) => p,
                Err(e) => {
                    let (rc, rs) = engine_to_dbus_err_tuple(&e);
                    return (default_return, rc, rs);
                }
            };
            (
                (true, OwnedObjectPath::from(path)),
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            )
        }
        Ok(Ok(CreateAction::Identity)) => (
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
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

pub async fn add_data_devs_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    pool_uuid: PoolUuid,
    devices: Vec<PathBuf>,
) -> ((bool, Vec<OwnedObjectPath>), u16, String) {
    let default_return = (false, Vec::default());

    let guard_res = engine
        .get_mut_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    let conn_clone = Arc::clone(connection);
    let man_clone = manager.clone();
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;
        let (name, _, pool) = guard.as_mut_tuple();
        let vec_path = devices.iter().map(|p| p.as_path()).collect::<Vec<_>>();
        let res = pool.add_blockdevs(
            pool_uuid,
            name.to_string().as_str(),
            vec_path.as_slice(),
            BlockDevTier::Data,
        );
        let _ = handle_action!(
            res.as_ref().map(|(action, _)| action.clone()),
            conn_clone,
            man_clone,
            pool_uuid
        );
        res
    })
    .await
    {
        Ok(Ok((action, diff))) => match action.changed() {
            Some(bd_uuids) => {
                if let Some(d) = diff {
                    send_pool_foreground_signals(connection, manager, pool_uuid, d).await;
                }
                let mut bd_paths = Vec::new();
                for dev_uuid in bd_uuids {
                    match register_blockdev(
                        engine, connection, manager, counter, pool_uuid, dev_uuid,
                    )
                    .await
                    {
                        Ok(op) => bd_paths.push(op.into()),
                        Err(_) => {
                            warn!("Unable to register object path for blockdev with UUID {dev_uuid} belonging to pool {pool_uuid} on the D-Bus");
                        }
                    }
                }
                (
                    (true, bd_paths),
                    DbusErrorEnum::OK as u16,
                    OK_STRING.to_string(),
                )
            }
            None => (
                default_return,
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            ),
        },
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

pub async fn init_cache_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    pool_uuid: PoolUuid,
    devices: Vec<PathBuf>,
) -> ((bool, Vec<OwnedObjectPath>), u16, String) {
    let default_return = (false, Vec::default());

    let guard_res = engine
        .get_mut_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    let conn_clone = Arc::clone(connection);
    let man_clone = manager.clone();
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;
        let (name, _, pool) = guard.as_mut_tuple();
        let vec_path = devices.iter().map(|p| p.as_path()).collect::<Vec<_>>();
        handle_action!(
            pool.init_cache(
                pool_uuid,
                name.to_string().as_str(),
                vec_path.as_slice(),
                false,
            ),
            conn_clone,
            man_clone,
            pool_uuid
        )
    })
    .await
    {
        Ok(Ok(action)) => match action.changed() {
            Some(bd_uuids) => {
                match manager.read().await.pool_get_path(&pool_uuid) {
                    Some(p) => {
                        send_has_cache_signal(connection, p).await;
                    }
                    None => {
                        warn!("No object path associated with pool UUID {pool_uuid}; failed to send pool has cache change signals");
                    }
                };
                let mut bd_paths = Vec::new();
                for dev_uuid in bd_uuids {
                    match register_blockdev(
                        engine, connection, manager, counter, pool_uuid, dev_uuid,
                    )
                    .await
                    {
                        Ok(op) => bd_paths.push(op.into()),
                        Err(_) => {
                            warn!("Unable to register object path for blockdev with UUID {dev_uuid} belonging to pool {pool_uuid} on the D-Bus");
                        }
                    }
                }
                (
                    (true, bd_paths),
                    DbusErrorEnum::OK as u16,
                    OK_STRING.to_string(),
                )
            }
            None => (
                default_return,
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            ),
        },
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

pub async fn add_cache_devs_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    pool_uuid: PoolUuid,
    devices: Vec<PathBuf>,
) -> ((bool, Vec<OwnedObjectPath>), u16, String) {
    let default_return = (false, Vec::default());

    let guard_res = engine
        .get_mut_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    let conn_clone = Arc::clone(connection);
    let man_clone = manager.clone();
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;
        let (name, _, pool) = guard.as_mut_tuple();
        let vec_path = devices.iter().map(|p| p.as_path()).collect::<Vec<_>>();
        handle_action!(
            pool.add_blockdevs(
                pool_uuid,
                name.to_string().as_str(),
                vec_path.as_slice(),
                BlockDevTier::Cache,
            )
            .map(|(action, _)| action),
            conn_clone,
            man_clone,
            pool_uuid
        )
    })
    .await
    {
        Ok(Ok(action)) => match action.changed() {
            Some(bd_uuids) => {
                let mut bd_paths = Vec::new();
                for dev_uuid in bd_uuids {
                    match register_blockdev(
                        engine, connection, manager, counter, pool_uuid, dev_uuid,
                    )
                    .await
                    {
                        Ok(op) => bd_paths.push(op.into()),
                        Err(_) => {
                            warn!("Unable to register object path for blockdev with UUID {dev_uuid} belonging to pool {pool_uuid} on the D-Bus");
                        }
                    }
                }
                (
                    (true, bd_paths),
                    DbusErrorEnum::OK as u16,
                    OK_STRING.to_string(),
                )
            }
            None => (
                default_return,
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            ),
        },
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

pub async fn set_name_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
    name: &str,
) -> ((bool, String), u16, String) {
    let default_return = (false, PoolUuid::default().simple().to_string());

    match engine.rename_pool(pool_uuid, name).await {
        Ok(RenameAction::NoSource) => {
            let (rc, rs) = (
                DbusErrorEnum::ERROR as u16,
                format!("engine doesn't know about pool {pool_uuid}"),
            );
            (default_return, rc, rs)
        }
        Ok(RenameAction::Identity) => (
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Ok(RenameAction::Renamed(uuid)) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_pool_name_signal(connection, &p.as_ref()).await;
                }
                None => {
                    warn!("No object path associated with pool UUID {uuid}; failed to send pool name change signals");
                }
            };
            (
                (true, pool_uuid.simple().to_string()),
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

pub async fn bind_clevis_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
    pin: String,
    json: &str,
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
        let free_token_slots = pool.free_token_slots();
        let action = handle_action!(
            pool.bind_clevis(
                &name,
                OptionalTokenSlotInput::Legacy,
                pin.as_str(),
                &json_value,
            ),
            conn_clone,
            man_clone,
            pool_uuid
        );
        let new_free_token_slots = pool.free_token_slots();
        match action {
            Ok(CreateAction::Created(_)) => Ok(CreateAction::Created((
                free_token_slots,
                new_free_token_slots,
            ))),
            Ok(CreateAction::Identity) => Ok(CreateAction::Identity),
            Err(e) => Err(e),
        }
    })
    .await
    {
        Ok(Ok(CreateAction::Identity)) => (false, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Ok(Ok(CreateAction::Created((fts, nfts)))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_clevis_info_signal(connection, p, true).await;
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
        let free_token_slots = pool.free_token_slots();
        let action = handle_action!(
            pool.bind_keyring(&name, OptionalTokenSlotInput::Legacy, &kd),
            conn_clone,
            man_clone,
            pool_uuid
        );
        let new_free_token_slots = pool.free_token_slots();
        match action {
            Ok(CreateAction::Created(_)) => Ok(CreateAction::Created((
                free_token_slots,
                new_free_token_slots,
            ))),
            Ok(CreateAction::Identity) => Ok(CreateAction::Identity),
            Err(e) => Err(e),
        }
    })
    .await
    {
        Ok(Ok(CreateAction::Identity)) => (false, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Ok(Ok(CreateAction::Created((fts, nfts)))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_keyring_signal(connection, p, true).await;
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
        handle_action!(guard.rebind_clevis(None), conn_clone, man_clone, pool_uuid)
    })
    .await
    {
        Ok(Ok(_)) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => send_clevis_info_signal(connection, p, true).await,
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
        handle_action!(
            guard.rebind_keyring(None, &key_desc),
            conn_clone,
            man_clone,
            pool_uuid
        )
    })
    .await
    {
        Ok(Ok(RenameAction::Renamed(_))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => send_keyring_signal(connection, p, true).await,
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
        handle_action!(
            pool.unbind_clevis(&name, None),
            conn_clone,
            man_clone,
            pool_uuid
        )
    })
    .await
    {
        Ok(Ok(DeleteAction::Deleted(_))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => send_clevis_info_signal(connection, p, true).await,
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
        handle_action!(
            pool.unbind_keyring(&name, None),
            conn_clone,
            man_clone,
            pool_uuid
        )
    })
    .await
    {
        Ok(Ok(DeleteAction::Deleted(_))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => send_keyring_signal(connection, p, true).await,
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

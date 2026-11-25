// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc},
};

use futures::executor::block_on;
use serde_json::from_str;
use tokio::sync::RwLock;
use zbus::{
    zvariant::{ObjectPath, OwnedObjectPath},
    Connection,
};

use devicemapper::Bytes;

use crate::{
    dbus::{
        blockdev::{register_blockdev, unregister_blockdev},
        consts::OK_STRING,
        filesystem::{register_filesystem, unregister_filesystem},
        manager::Manager,
        types::{DbusErrorEnum, FilesystemSpec},
        util::{
            engine_to_dbus_err_tuple, send_blockdev_physical_size_signal, send_clevis_info_signal,
            send_encrypted_signal, send_free_token_slots_signal, send_has_cache_signal,
            send_keyring_signal, send_last_reencrypted_signal, send_origin_signal,
            send_pool_foreground_signals, send_pool_name_signal, tuple_to_option,
        },
    },
    engine::{
        BlockDevTier, CreateAction, DeleteAction, DevUuid, EncryptedDevice, Engine, EngineAction,
        InputEncryptionInfo, KeyDescription, Lockable, OptionalTokenSlotInput, PoolIdentifier,
        PoolUuid, RenameAction, StratSigblockVersion,
    },
    stratis::StratisError,
};

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
            Some((changed, updated_origins)) => {
                for uuid in changed.iter() {
                    let opt = manager.read().await.filesystem_get_path(uuid).cloned();
                    match opt {
                        Some(p) => {
                            if let Err(e) =
                                unregister_filesystem(connection, manager, &p.as_ref()).await
                            {
                                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                                return (default_return, rc, rs);
                            }
                        }
                        None => {
                            warn!("No path found to unregister for destroyed filesystem with UUID {uuid}");
                        }
                    }
                }
                for (updated_origin, _) in updated_origins {
                    match manager
                        .read()
                        .await
                        .filesystem_get_path(&updated_origin)
                        .cloned()
                    {
                        Some(p) => send_origin_signal(connection, &p.as_ref()).await,
                        None => {
                            warn!("Failed to find path associated with filesystem UUID {updated_origin}");
                        }
                    }
                }
                (
                    (
                        true,
                        changed
                            .into_iter()
                            .map(|u| u.simple().to_string())
                            .collect(),
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

pub async fn set_name_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
    name: &str,
) -> ((bool, String), u16, String) {
    let default_return = (false, String::new());

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
            let read_lock = manager.read().await;
            match read_lock.pool_get_path(&pool_uuid) {
                Some(p) => match engine.get_pool(PoolIdentifier::Uuid(pool_uuid)).await {
                    Some(pool) => {
                        let fs_paths = pool.filesystems().into_iter().filter_map(|(_, fs_uuid, _)| match read_lock.filesystem_get_path(&fs_uuid) {
                                Some(p) => Some(p),
                                None => {
                                    warn!("Could not find filesystem path for filesystem with UUID {fs_uuid}; cannot send devnode invalidate signal");
                                    None
                                }
                            })
                            .collect::<Vec<_>>();
                        send_pool_name_signal(connection, &p.as_ref(), fs_paths).await;
                    }
                    None => {
                        warn!("Could not find filesystems associated with pool with UUID {uuid}; cannot send devnode invalidate signal");
                        send_pool_name_signal(connection, &p.as_ref(), vec![]).await;
                    }
                },
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

pub async fn grow_physical_device_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
    dev: &str,
) -> (bool, u16, String) {
    let default_return = false;

    let dev = match DevUuid::parse_str(dev) {
        Ok(u) => u,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
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
        let res = pool.grow_physical(&name, pool_uuid, dev);
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
            Some((_, dev_uuid)) => {
                if let Some(d) = diff {
                    send_pool_foreground_signals(connection, manager, pool_uuid, d).await;
                    match manager.read().await.blockdev_get_path(&dev_uuid) {
                        Some(p) => {
                            send_blockdev_physical_size_signal(connection, p).await;
                        }
                        None => {
                            warn!("No path found for blockdev with UUID {dev_uuid}");
                        }
                    }
                }
                (true, DbusErrorEnum::OK as u16, OK_STRING.to_string())
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
            pool.init_cache(pool_uuid, name.to_string().as_str(), vec_path.as_slice(),),
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

pub async fn create_filesystems_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    pool_uuid: PoolUuid,
    filesystems: FilesystemSpec<'_>,
) -> ((bool, Vec<(OwnedObjectPath, String)>), u16, String) {
    let default_return = (false, (Vec::new()));

    if filesystems.len() > 1 {
        return (
            default_return,
            DbusErrorEnum::ERROR as u16,
            "Currently filesystem creation is limited to one filesystem at a time".to_string(),
        );
    }

    let filesystem_specs = match filesystems
        .into_iter()
        .map(|(name, size_opt, size_limit_opt)| {
            let size = tuple_to_option(size_opt)
                .map(|val| {
                    val.parse::<u128>().map_err(|_| {
                        format!("Could not parse filesystem size string {val} to integer value")
                    })
                })
                .transpose()?;
            let size_limit = tuple_to_option(size_limit_opt)
                .map(|val| {
                    val.parse::<u128>().map_err(|_| {
                        format!(
                            "Could not parse filesystem size limit string {val} to integer value"
                        )
                    })
                })
                .transpose()?;
            Ok((name.to_string(), size.map(Bytes), size_limit.map(Bytes)))
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
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;
        let (name, _, pool) = guard.as_mut_tuple();
        handle_action!(pool.create_filesystems(
            name.to_string().as_str(),
            pool_uuid,
            filesystem_specs
                .iter()
                .map(|(s, b1, b2)| (s.as_str(), *b1, *b2))
                .collect::<Vec<_>>()
                .as_slice(),
        ))
    })
    .await
    {
        Ok(Ok(changed)) => {
            let mut object_paths = Vec::new();
            match changed.changed() {
                Some(v) => {
                    for (name, uuid, _) in v {
                        match register_filesystem(
                            engine, connection, manager, counter, pool_uuid, uuid,
                        )
                        .await
                        {
                            Ok(path) => {
                                object_paths.push((OwnedObjectPath::from(path), name.to_string()));
                            }
                            Err(e) => {
                                warn!("Failed to register the filesystem with the D-Bus: {e}; object may not be visible to clients");
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

pub async fn filesystem_metadata_method(
    engine: &Arc<dyn Engine>,
    pool_uuid: PoolUuid,
    fs_name: (bool, &str),
    current: bool,
) -> (String, u16, String) {
    let default_return = String::new();

    let fs_name_opt = tuple_to_option(fs_name).map(|f| f.to_owned());

    let guard_res = engine
        .get_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    match tokio::task::spawn_blocking(move || {
        let guard = guard_res?;
        if current {
            guard.current_fs_metadata(fs_name_opt.as_deref())
        } else {
            guard.last_fs_metadata(fs_name_opt.as_deref())
        }
    })
    .await
    {
        Ok(Ok(metadata)) => (metadata, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
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

pub async fn metadata_method(
    engine: &Arc<dyn Engine>,
    pool_uuid: PoolUuid,
    current: bool,
) -> (String, u16, String) {
    let default_return = String::new();

    let guard_res = engine
        .get_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    match tokio::task::spawn_blocking(move || {
        let guard = guard_res?;
        let (name, _, pool) = guard.as_tuple();
        if current {
            pool.current_metadata(&name)
        } else {
            pool.last_metadata()
        }
    })
    .await
    {
        Ok(Ok(metadata)) => (metadata, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
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
                    send_clevis_info_signal(connection, p).await;
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
                    send_keyring_signal(connection, p).await;
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

        handle_action!(
            guard.rebind_clevis(token_slot),
            conn_clone,
            man_clone,
            pool_uuid
        )
        .map(|_| ())
    })
    .await
    {
        Ok(Ok(())) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => send_clevis_info_signal(connection, p).await,
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

        handle_action!(
            guard.rebind_keyring(token_slot, &key_desc),
            conn_clone,
            man_clone,
            pool_uuid
        )
        .map(|action| match action {
            RenameAction::Renamed(_) => RenameAction::Renamed(()),
            RenameAction::Identity => RenameAction::Identity,
            RenameAction::NoSource => RenameAction::NoSource,
        })
    })
    .await
    {
        Ok(Ok(RenameAction::Renamed(()))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => send_keyring_signal(connection, p).await,
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

        handle_action!(
            pool.unbind_clevis(&name, token_slot),
            conn_clone,
            man_clone,
            pool_uuid
        )
        .map(|action| match action {
            DeleteAction::Deleted(_) => DeleteAction::Deleted(()),
            DeleteAction::Identity => DeleteAction::Identity,
        })
    })
    .await
    {
        Ok(Ok(DeleteAction::Deleted(()))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_clevis_info_signal(connection, p).await;
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

        handle_action!(
            pool.unbind_keyring(&name, token_slot),
            conn_clone,
            man_clone,
            pool_uuid
        )
        .map(|action| match action {
            DeleteAction::Deleted(_) => DeleteAction::Deleted(()),
            DeleteAction::Identity => DeleteAction::Identity,
        })
    })
    .await
    {
        Ok(Ok(DeleteAction::Deleted(()))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_keyring_signal(connection, p).await;
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
    let cloned_engine = Arc::clone(engine);
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;

        handle_action!(match guard.decrypt_pool_idem_check(pool_uuid) {
            Ok(DeleteAction::Identity) => Ok(DeleteAction::Identity),
            Ok(DeleteAction::Deleted(d)) => {
                let guard = guard.downgrade();
                guard
                    .do_decrypt_pool(pool_uuid)
                    .and_then(|_| {
                        let mut guard = block_on(cloned_engine.upgrade_pool(guard));
                        let (name, _, _) = guard.as_mut_tuple();
                        guard.finish_decrypt_pool(pool_uuid, &name)
                    })
                    .map(|_| DeleteAction::Deleted(d))
            }
            Err(e) => Err(e),
        })
    })
    .await
    {
        Ok(Ok(DeleteAction::Deleted(_))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_keyring_signal(connection, &p.as_ref()).await;
                    send_clevis_info_signal(connection, &p.as_ref()).await;
                    send_encrypted_signal(connection, &p.as_ref()).await;
                    send_last_reencrypted_signal(connection, &p.as_ref()).await;
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
    let cloned_engine = Arc::clone(engine);
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;

        handle_action!(guard
            .start_encrypt_pool(pool_uuid, &iei)
            .and_then(|action| match action {
                CreateAction::Identity => Ok(CreateAction::Identity),
                CreateAction::Created((sector_size, key_info)) => {
                    let guard = guard.downgrade();
                    guard
                        .do_encrypt_pool(pool_uuid, sector_size, key_info)
                        .map(|_| guard)
                        .and_then(|guard| {
                            let mut guard = block_on(cloned_engine.upgrade_pool(guard));
                            let (name, _, _) = guard.as_mut_tuple();
                            guard.finish_encrypt_pool(&name, pool_uuid)
                        })
                        .map(|_| CreateAction::Created(EncryptedDevice(pool_uuid)))
                }
            }))
    })
    .await
    {
        Ok(Ok(CreateAction::Created(_))) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_keyring_signal(connection, &p.as_ref()).await;
                    send_clevis_info_signal(connection, &p.as_ref()).await;
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
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
) -> (bool, u16, String) {
    let default_return = false;

    let guard_res = engine
        .get_mut_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    let cloned_engine = Arc::clone(engine);
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;

        let (name, _, _) = guard.as_mut_tuple();

        let result = guard.start_reencrypt_pool();
        let result = result.and_then(|key_info| {
            let guard = guard.downgrade();
            let result = guard.do_reencrypt_pool(pool_uuid, key_info);
            result.map(|inner| (guard, inner))
        });
        let result = result.and_then(|(guard, _)| {
            let mut guard = block_on(cloned_engine.upgrade_pool(guard));
            guard.finish_reencrypt_pool(&name, pool_uuid)
        });
        handle_action!(result)
    })
    .await
    {
        Ok(Ok(_)) => {
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_last_reencrypted_signal(connection, &p.as_ref()).await;
                }
                None => {
                    warn!("No pool path associated with UUID {pool_uuid}; failed to send encryption related signals");
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

pub async fn remove_cache_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
) -> ((bool, Vec<String>), u16, String) {
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
        handle_action!(
            pool.remove_cache(pool_uuid, name.to_string().as_str()),
            conn_clone,
            man_clone,
            pool_uuid
        )
    })
    .await
    {
        Ok(Ok(action)) => match action.changed() {
            Some((dev_uuids, _)) => {
                match manager.read().await.pool_get_path(&pool_uuid) {
                    Some(p) => {
                        send_has_cache_signal(connection, p).await;
                    }
                    None => {
                        warn!("No object path associated with pool UUID {pool_uuid}; failed to send pool has cache change signals");
                    }
                };

                let mut removed_uuids = Vec::new();
                let mut failed_unregisters = Vec::new();
                for dev_uuid in dev_uuids {
                    let opt = manager.read().await.blockdev_get_path(&dev_uuid).cloned();
                    match opt {
                        Some(p) => {
                            if let Err(e) =
                                unregister_blockdev(connection, manager, &p.as_ref()).await
                            {
                                failed_unregisters.push(e);
                            } else {
                                removed_uuids.push(dev_uuid.simple().to_string());
                            }
                        }
                        None => {
                            warn!("No path found to unregister for removed cache blockdev with UUID {dev_uuid}");
                        }
                    }
                }
                if failed_unregisters.is_empty() {
                    (
                        (true, removed_uuids),
                        DbusErrorEnum::OK as u16,
                        OK_STRING.to_string(),
                    )
                } else {
                    let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::BestEffortError(
                        "Failed to unregister all blockdevs from the D-Bus".to_string(),
                        failed_unregisters,
                    ));
                    (default_return, rc, rs)
                }
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

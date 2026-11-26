// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::HashSet,
    os::fd::AsRawFd,
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc},
};

use serde_json::{from_str, Value};
use tokio::sync::RwLock;
use zbus::{
    zvariant::{Fd, ObjectPath, OwnedObjectPath},
    Connection,
};

use devicemapper::Bytes;

use crate::{
    dbus::{
        consts::OK_STRING,
        filesystem::unregister_filesystem,
        manager::Manager,
        pool::{register_pool, unregister_pool},
        types::DbusErrorEnum,
        util::{engine_to_dbus_err_tuple, send_stopped_pools_signals, tuple_to_option},
    },
    engine::{
        CreateAction, DeleteAction, Engine, InputEncryptionInfo, IntegritySpec, IntegrityTagSpec,
        KeyDescription, Lockable, MappingCreateAction, MappingDeleteAction, Name, PoolIdentifier,
        PoolUuid, StartAction, StopAction, TokenUnlockMethod,
    },
    stratis::{StratisError, StratisResult},
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

    let (dev_uuids, fs_uuids) = match engine.get_pool(PoolIdentifier::Uuid(uuid)).await {
        Some(p) => (
            p.blockdevs().into_iter().map(|(u, _, _)| u).collect(),
            p.filesystems().into_iter().map(|(_, u, _)| u).collect(),
        ),
        None => (vec![], vec![]),
    };

    match engine.destroy_pool(uuid).await {
        Ok(DeleteAction::Deleted(uuid)) => {
            match unregister_pool(connection, manager, &pool, &fs_uuids, &dev_uuids).await {
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

pub fn engine_state_report_method(engine: &Arc<dyn Engine>) -> (String, u16, String) {
    match serde_json::to_string(&engine.engine_state_report()) {
        Ok(result) => (result, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e.into());
            (String::new(), rc, rs)
        }
    }
}

pub async fn refresh_state_method(engine: &Arc<dyn Engine>) -> (u16, String) {
    match engine.refresh_state().await {
        Ok(_) => (DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Err(e) => engine_to_dbus_err_tuple(&e),
    }
}

pub async fn stop_pool_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    id: &str,
    id_type: &str,
) -> ((bool, String), u16, String) {
    let default_return = (false, String::new());

    let id = match id_type {
        "uuid" => match PoolUuid::parse_str(id) {
            Ok(u) => PoolIdentifier::Uuid(u),
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                return (default_return, rc, rs);
            }
        },
        "name" => PoolIdentifier::Name(Name::new(id.to_string())),
        _ => {
            return (
                default_return,
                DbusErrorEnum::ERROR as u16,
                format!("ID type {id_type} not recognized"),
            );
        }
    };

    let (dev_uuids, fs_uuids) = match engine.get_pool(id.clone()).await {
        Some(p) => (
            p.blockdevs().into_iter().map(|(u, _, _)| u).collect(),
            p.filesystems().into_iter().map(|(_, u, _)| u).collect(),
        ),
        None => (vec![], vec![]),
    };

    let action = handle_action!(engine.stop_pool(id.clone()).await);

    match &action {
        Ok(StopAction::Stopped(pool_uuid) | StopAction::Partial(pool_uuid)) => {
            let path = manager.read().await.pool_get_path(pool_uuid).cloned();
            match path {
                Some(pool) => {
                    if let Err(e) =
                        unregister_pool(connection, manager, &pool.as_ref(), &fs_uuids, &dev_uuids)
                            .await
                    {
                        warn!("Failed to remove pool with path {pool} from the D-Bus: {e}");
                    }
                }
                None => {
                    warn!("Failed to unregister the stopped pool from the D-Bus");
                }
            };
            send_stopped_pools_signals(connection).await;
        }
        Err(_) => match engine.get_pool(id.clone()).await {
            Some(g) => {
                let rem_fs = g
                    .filesystems()
                    .into_iter()
                    .map(|(_, uuid, _)| uuid)
                    .collect::<HashSet<_>>();
                for fs_uuid in fs_uuids.iter().filter(|u| !rem_fs.contains(u)) {
                    match manager.read().await.filesystem_get_path(fs_uuid).cloned() {
                        Some(p) => {
                            if let Err(e) = unregister_filesystem(connection, manager, &p).await {
                                warn!("Failed to unregister filesystem: {e}");
                            }
                        }
                        None => {
                            warn!("Could not find filesystem path for UUID {fs_uuid}");
                        }
                    }
                }
            }
            None => {
                warn!("Failed to find pool with ID {id:?} even though pool failed to stop");
            }
        },
        Ok(StopAction::Identity | StopAction::CleanedUp(_)) => {}
    }

    match action {
        Ok(StopAction::Identity) => (
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Ok(StopAction::Stopped(pool_uuid) | StopAction::CleanedUp(pool_uuid)) => (
            (true, pool_uuid.simple().to_string()),
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Ok(StopAction::Partial(_)) => (
            default_return,
            DbusErrorEnum::ERROR as u16,
            "Pool was stopped, but some component devices were not torn down".to_string(),
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
    key_desc: Vec<((bool, u32), KeyDescription)>,
    clevis_info: Vec<((bool, u32), &str, &str)>,
    journal_size: (bool, u64),
    tag_spec: (bool, &str),
    allocate_superblock: (bool, bool),
) -> ((bool, (OwnedObjectPath, Vec<OwnedObjectPath>)), u16, String) {
    let default_return = (false, (OwnedObjectPath::default(), Vec::new()));

    let devs_ref = devs.iter().map(|path| path.as_path()).collect::<Vec<_>>();
    let key_desc = key_desc
        .into_iter()
        .map(|(tup, kd)| (tuple_to_option(tup), kd))
        .collect::<Vec<_>>();
    let clevis_info = match clevis_info.into_iter().try_fold::<_, _, StratisResult<_>>(
        Vec::new(),
        |mut vec, (tup, s, json)| {
            vec.push((
                tuple_to_option(tup),
                (
                    s.to_string(),
                    from_str::<Value>(json).map_err(StratisError::from)?,
                ),
            ));
            Ok(vec)
        },
    ) {
        Ok(ci) => ci,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return (default_return, rc, rs);
        }
    };
    let iei = match InputEncryptionInfo::new(key_desc, clevis_info) {
        Ok(info) => info,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return (default_return, rc, rs);
        }
    };
    let journal_size = tuple_to_option(journal_size).map(Bytes::from);
    let tag_spec = match tuple_to_option(tag_spec)
        .map(IntegrityTagSpec::try_from)
        .transpose()
    {
        Ok(s) => s,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Msg(format!(
                "Failed to parse integrity tag specification: {e}"
            )));
            return (default_return, rc, rs);
        }
    };
    let allocate_superblock = tuple_to_option(allocate_superblock);

    match handle_action!(
        engine
            .create_pool(
                name,
                devs_ref.as_slice(),
                iei.as_ref(),
                IntegritySpec {
                    journal_size,
                    tag_spec,
                    allocate_superblock,
                },
            )
            .await
    ) {
        Ok(CreateAction::Created(uuid)) => {
            match register_pool(engine, connection, manager, counter, uuid).await {
                Ok((pool_path, _, bd_paths)) => (
                    (
                        true,
                        (
                            OwnedObjectPath::from(pool_path),
                            bd_paths.into_iter().map(OwnedObjectPath::from).collect(),
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

#[allow(clippy::too_many_arguments)]
pub async fn start_pool_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    id: &str,
    id_type: &str,
    unlock_method_tuple: (bool, (bool, u32)),
    key_fd: (bool, Fd<'_>),
    remove_cache: bool,
) -> (
    (
        bool,
        (OwnedObjectPath, Vec<OwnedObjectPath>, Vec<OwnedObjectPath>),
    ),
    u16,
    String,
) {
    let default_return = (
        false,
        (OwnedObjectPath::default(), Vec::default(), Vec::default()),
    );

    let id = match id_type {
        "uuid" => match PoolUuid::parse_str(id) {
            Ok(u) => PoolIdentifier::Uuid(u),
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                return (default_return, rc, rs);
            }
        },
        "name" => PoolIdentifier::Name(Name::new(id.to_string())),
        _ => {
            return (
                default_return,
                DbusErrorEnum::ERROR as u16,
                format!("ID type {id_type} not recognized"),
            );
        }
    };
    let key_fd_option = tuple_to_option(key_fd);

    match handle_action!(
        engine
            .start_pool(
                id,
                TokenUnlockMethod::from_options(
                    tuple_to_option(unlock_method_tuple).map(tuple_to_option)
                ),
                key_fd_option.as_ref().map(|fd| fd.as_raw_fd()),
                remove_cache,
            )
            .await
    ) {
        Ok(StartAction::Started(pool_uuid)) => {
            if engine
                .get_pool(PoolIdentifier::Uuid(pool_uuid))
                .await
                .is_none()
            {
                return (
                    default_return,
                    DbusErrorEnum::ERROR as u16,
                    format!("No pool found for newly started pool with UUID {pool_uuid}"),
                );
            }
            let (pool_path, fs_paths, dev_paths) =
                match register_pool(engine, connection, manager, counter, pool_uuid).await {
                    Ok((pp, fp, dp)) => (
                        OwnedObjectPath::from(pp),
                        fp.into_iter().map(OwnedObjectPath::from).collect(),
                        dp.into_iter().map(OwnedObjectPath::from).collect(),
                    ),
                    Err(e) => {
                        let (rc, rs) = engine_to_dbus_err_tuple(&e);
                        return (default_return, rc, rs);
                    }
                };

            send_stopped_pools_signals(connection).await;

            (
                (true, (pool_path, dev_paths, fs_paths)),
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            )
        }
        Ok(StartAction::Identity) => (
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

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    path::PathBuf,
    os::fd::AsRawFd,
    sync::{atomic::AtomicU64, Arc},
};

use serde_json::{from_str, Value};
use tokio::sync::RwLock;
use zbus::{
    zvariant::{Fd, ObjectPath},
    Connection,
};

use devicemapper::Bytes;

use crate::{
    dbus::{
        consts::OK_STRING,
        manager::Manager,
        filesystem::register_filesystem,
        pool::register_pool,
        types::DbusErrorEnum,
        util::{engine_to_dbus_err_tuple, tuple_to_option, send_stopped_pools_signals, send_locked_pools_signals},
    },
    engine::{
        CreateAction, Engine, InputEncryptionInfo, IntegritySpec, IntegrityTagSpec, KeyDescription,
        Lockable, Name, PoolIdentifier, PoolUuid, StartAction, TokenUnlockMethod,
    },
    stratis::{StratisError, StratisResult},
};

#[allow(clippy::too_many_arguments)]
pub async fn create_pool_method<'a>(
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
) -> ((bool, (ObjectPath<'a>, Vec<ObjectPath<'a>>)), u16, String) {
    let default_return = (false, (ObjectPath::default(), Vec::new()));

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
                Ok(tuple) => (
                    (true, tuple),
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
pub async fn start_pool_method<'a>(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    id: &str,
    id_type: &str,
    unlock_method_tuple: (bool, (bool, u32)),
    key_fd: (bool, Fd<'_>),
) -> (
    (
        bool,
        (ObjectPath<'a>, Vec<ObjectPath<'a>>, Vec<ObjectPath<'a>>),
    ),
    u16,
    String,
) {
    let default_return = (
        false,
        (ObjectPath::default(), Vec::default(), Vec::default()),
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
                false
            )
            .await
    ) {
        Ok(StartAction::Started(pool_uuid)) => {
            let guard = match engine.get_pool(PoolIdentifier::Uuid(pool_uuid)).await {
                Some(g) => g,
                None => {
                    return (
                        default_return,
                        DbusErrorEnum::ERROR as u16,
                        format!("No pool found for newly started pool with UUID {pool_uuid}"),
                    );
                }
            };
            let mut fs_paths = Vec::default();
            for fs_uuid in guard
                .filesystems()
                .into_iter()
                .map(|(_, fs_uuid, _)| fs_uuid)
                .collect::<Vec<_>>()
            {
                let fs_path = match register_filesystem(
                    engine, connection, manager, counter, pool_uuid, fs_uuid,
                )
                .await
                {
                    Ok(fp) => fp,
                    Err(e) => {
                        let (rc, rs) = engine_to_dbus_err_tuple(&e);
                        return (default_return, rc, rs);
                    }
                };
                fs_paths.push(fs_path);
            }
            let (pool_path, dev_paths) =
                match register_pool(engine, connection, manager, counter, pool_uuid).await {
                    Ok(pp) => pp,
                    Err(e) => {
                        let (rc, rs) = engine_to_dbus_err_tuple(&e);
                        return (default_return, rc, rs);
                    }
                };

            if guard.is_encrypted() {
                if let Err(e) = send_locked_pools_signals(connection).await {
                    warn!(
                        "Failed to send signals for changed properties for the Manager interfaces: {e}"
                    );
                }
            }
            if let Err(e) = send_stopped_pools_signals(connection).await {
                warn!(
                    "Failed to send signals for changed properties for the Manager interfaces: {e}"
                );
            }

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

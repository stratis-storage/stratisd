// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    os::fd::AsRawFd,
    sync::{atomic::AtomicU64, Arc},
};

use tokio::sync::RwLock;
use zbus::{
    zvariant::{Fd, ObjectPath},
    Connection,
};

use crate::{
    dbus::{
        consts::OK_STRING,
        filesystem::register_filesystem,
        manager::Manager,
        pool::register_pool,
        types::DbusErrorEnum,
        util::{
            engine_to_dbus_err_tuple, send_locked_pools_signals, send_stopped_pools_signals,
            tuple_to_option,
        },
    },
    engine::{Engine, Lockable, Name, PoolIdentifier, PoolUuid, StartAction, TokenUnlockMethod},
};

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
    remove_cache: bool,
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
                remove_cache,
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

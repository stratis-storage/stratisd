// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{atomic::AtomicU64, Arc};

use tokio::sync::RwLock;
use zbus::{zvariant::ObjectPath, Connection};

use crate::{
    dbus::{
        consts::OK_STRING,
        filesystem::register_filesystem,
        manager::Manager,
        pool::{register_pool, unregister_pool},
        types::DbusErrorEnum,
        util::{
            engine_to_dbus_err_tuple, send_locked_pools_signals, send_stopped_pools_signals,
            tuple_to_option,
        },
    },
    engine::{
        Engine, Lockable, PoolIdentifier, PoolUuid, StartAction, StopAction, TokenUnlockMethod,
        UnlockMethod,
    },
};

pub async fn start_pool_method<'a>(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    pool_uuid: PoolUuid,
    unlock_method_tuple: (bool, UnlockMethod),
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

    let unlock_method = tuple_to_option(unlock_method_tuple);

    match handle_action!(
        engine
            .start_pool(
                PoolIdentifier::Uuid(pool_uuid),
                TokenUnlockMethod::from(unlock_method),
                None,
                false
            )
            .await
    ) {
        Ok(StartAction::Started(_)) => {
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

pub async fn stop_pool_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool: ObjectPath<'_>,
) -> ((bool, PoolUuid), u16, String) {
    let default_return = (false, PoolUuid::default());

    let pool_uuid = match manager.read().await.pool_get_uuid(&pool) {
        Some(u) => u,
        None => {
            return (
                default_return,
                DbusErrorEnum::ERROR as u16,
                format!("No pool found in engine associated with object path {pool}"),
            );
        }
    };

    let send_locked_signal = engine
        .get_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .map(|g| g.is_encrypted())
        .unwrap_or(false);

    let action = handle_action!(
        engine
            .stop_pool(PoolIdentifier::Uuid(pool_uuid), false)
            .await
    );

    if let Ok(StopAction::Stopped(_) | StopAction::Partial(_)) = action {
        if let Err(e) = unregister_pool(connection, manager, &pool).await {
            warn!("Failed to remove pool with path {pool} from the D-Bus: {e}");
        }
        if let Err(e) = send_stopped_pools_signals(connection).await {
            warn!("Failed to send signals for changed properties for the Manager interfaces: {e}");
        }
        if send_locked_signal {
            if let Err(e) = send_locked_pools_signals(connection).await {
                warn!(
                    "Failed to send signals for changed properties for the Manager interfaces: {e}"
                );
            }
        }
    }

    match action {
        Ok(StopAction::Identity) => (
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Ok(StopAction::Stopped(_)) => (
            (true, pool_uuid),
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Ok(StopAction::Partial(_)) => (
            default_return,
            DbusErrorEnum::ERROR as u16,
            "Pool was stopped, but some component devices were not torn down".to_string(),
        ),
        Ok(StopAction::CleanedUp(_)) => unreachable!("!has_partially_constructed above"),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            (default_return, rc, rs)
        }
    }
}

pub async fn refresh_state_method(engine: &Arc<dyn Engine>) -> (u16, String) {
    match engine.refresh_state().await {
        Ok(_) => (DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Err(e) => engine_to_dbus_err_tuple(&e),
    }
}

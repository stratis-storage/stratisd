// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{atomic::AtomicU64, Arc};

use tokio::sync::RwLock;
use zbus::{
    zvariant::{ObjectPath, OwnedObjectPath},
    Connection,
};

use crate::{
    dbus::{
        blockdev::unregister_blockdev,
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

pub async fn start_pool_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    pool_uuid: PoolUuid,
    unlock_method_tuple: (bool, UnlockMethod),
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
                fs_paths.push(OwnedObjectPath::from(fs_path));
            }
            let (pool_path, dev_paths) =
                match register_pool(engine, connection, manager, counter, pool_uuid).await {
                    Ok((pp, dp)) => (
                        OwnedObjectPath::from(pp),
                        dp.into_iter().map(OwnedObjectPath::from).collect(),
                    ),
                    Err(e) => {
                        let (rc, rs) = engine_to_dbus_err_tuple(&e);
                        return (default_return, rc, rs);
                    }
                };

            if guard.is_encrypted() {
                send_locked_pools_signals(connection).await;
            }
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

pub async fn stop_pool_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool: ObjectPath<'_>,
) -> ((bool, String), u16, String) {
    let default_return = (false, String::new());

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

    let dev_uuids = match engine.get_pool(PoolIdentifier::Uuid(pool_uuid)).await {
        Some(p) => p.blockdevs().into_iter().map(|(u, _, _)| u).collect(),
        None => vec![],
    };

    let action = handle_action!(
        engine
            .stop_pool(PoolIdentifier::Uuid(pool_uuid), false)
            .await
    );

    if let Ok(StopAction::Stopped(_) | StopAction::Partial(_)) = action {
        for dev_uuid in dev_uuids {
            let maybe_dev_path = manager.write().await.blockdev_get_path(&dev_uuid).cloned();
            if let Some(dev_path) = maybe_dev_path {
                if let Err(e) = unregister_blockdev(connection, manager, &dev_path).await {
                    warn!("Failed to unregister {dev_path} representing blockdev {dev_uuid}: {e}");
                }
            }
        }
        if let Err(e) = unregister_pool(connection, manager, &pool).await {
            warn!("Failed to remove pool with path {pool} from the D-Bus: {e}");
        }
        send_stopped_pools_signals(connection).await;
        let stopped_pools = engine.stopped_pools().await;
        let stopped = stopped_pools
            .stopped
            .get(&pool_uuid)
            .or_else(|| stopped_pools.partially_constructed.get(&pool_uuid));
        if stopped.map(|s| s.info.is_some()).unwrap_or(false) {
            send_locked_pools_signals(connection).await;
        }
    }

    match action {
        Ok(StopAction::Identity) => (
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Ok(StopAction::Stopped(_)) => (
            (true, pool_uuid.simple().to_string()),
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

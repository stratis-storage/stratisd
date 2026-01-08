// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use tokio::sync::RwLock;
use zbus::Connection;

use crate::{
    dbus::{
        blockdev::unregister_blockdev,
        consts::OK_STRING,
        filesystem::unregister_filesystem,
        manager::Manager,
        pool::unregister_pool,
        types::DbusErrorEnum,
        util::{engine_to_dbus_err_tuple, send_locked_pools_signals, send_stopped_pools_signals},
    },
    engine::{Engine, Lockable, Name, PoolIdentifier, PoolUuid, StopAction},
};

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

    let action = handle_action!(engine.stop_pool(id, true).await);

    if let Ok(StopAction::Stopped(pool_uuid) | StopAction::Partial(pool_uuid)) = action {
        for dev_uuid in dev_uuids {
            let maybe_dev_path = manager.write().await.blockdev_get_path(&dev_uuid).cloned();
            if let Some(dev_path) = maybe_dev_path {
                if let Err(e) = unregister_blockdev(connection, manager, &dev_path).await {
                    warn!("Failed to unregister {dev_path} representing blockdev {dev_uuid}: {e}");
                }
            }
        }

        for fs_uuid in fs_uuids {
            let maybe_fs_path = manager.write().await.filesystem_get_path(&fs_uuid).cloned();
            if let Some(fs_path) = maybe_fs_path {
                if let Err(e) = unregister_filesystem(connection, manager, &fs_path).await {
                    warn!("Failed to unregister {fs_path} representing filesystem {fs_uuid}: {e}");
                }
            }
        }
        let path = manager.read().await.pool_get_path(&pool_uuid).cloned();
        match path {
            Some(pool) => {
                if let Err(e) = unregister_pool(connection, manager, &pool.as_ref()).await {
                    warn!("Failed to remove pool with path {pool} from the D-Bus: {e}");
                }
            }
            None => {
                warn!("Failed to unregister the stopped pool from the D-Bus");
            }
        };
        send_stopped_pools_signals(connection).await;
        let stopped = {
            let stopped_pools = engine.stopped_pools().await;
            stopped_pools
                .stopped
                .get(&pool_uuid)
                .or_else(|| stopped_pools.partially_constructed.get(&pool_uuid))
                .map(|s| s.info.is_some())
                .unwrap_or(false)
        };
        if stopped {
            send_locked_pools_signals(connection).await;
        }
    }

    match action {
        Ok(StopAction::Identity) => (
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Ok(StopAction::Stopped(pool_uuid)) => (
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

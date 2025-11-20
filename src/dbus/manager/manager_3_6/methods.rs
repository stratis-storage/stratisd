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
) -> ((bool, PoolUuid), u16, String) {
    let default_return = (false, PoolUuid::default());

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

    let action = handle_action!(engine.stop_pool(id, false).await);

    if let Ok(StopAction::Stopped(pool_uuid) | StopAction::Partial(pool_uuid)) = action {
        match manager.read().await.pool_get_path(&pool_uuid) {
            Some(pool) => {
                if let Err(e) = unregister_pool(connection, manager, pool).await {
                    warn!("Failed to remove pool with path {pool} from the D-Bus: {e}");
                }
            }
            None => {
                warn!("Failed to unregister the stopped pool from the D-Bus");
            }
        };
        if let Err(e) = send_stopped_pools_signals(connection).await {
            warn!("Failed to send signals for changed properties for the Manager interfaces: {e}");
        }
        let stopped_pools = engine.stopped_pools().await;
        let stopped = stopped_pools
            .stopped
            .get(&pool_uuid)
            .or_else(|| stopped_pools.partially_constructed.get(&pool_uuid));
        if stopped.map(|s| s.info.is_some()).unwrap_or(false) {
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
        Ok(StopAction::Stopped(pool_uuid)) => (
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

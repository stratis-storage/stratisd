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
        manager::Manager,
        types::DbusErrorEnum,
        util::{engine_to_dbus_err_tuple, send_has_cache_signal},
    },
    engine::{Engine, EngineAction, Lockable, PoolIdentifier, PoolUuid},
    stratis::StratisError,
};

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
                for dev_uuid in dev_uuids {
                    let opt = manager.read().await.blockdev_get_path(&dev_uuid).cloned();
                    match opt {
                        Some(p) => {
                            if let Err(e) =
                                unregister_blockdev(connection, manager, &p.as_ref()).await
                            {
                                warn!("Unable to unregister object path for blockdev with UUID {dev_uuid} belonging to pool {pool_uuid} on the D-Bus: {e}");
                            }
                        }
                        None => {
                            warn!("No path found to unregister for removed cache blockdev with UUID {dev_uuid}");
                        }
                    }
                    removed_uuids.push(dev_uuid.simple().to_string());
                }
                (
                    (true, removed_uuids),
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

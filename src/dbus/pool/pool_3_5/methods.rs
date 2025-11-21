// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc},
};

use tokio::sync::RwLock;
use zbus::{zvariant::OwnedObjectPath, Connection};

use crate::{
    dbus::{
        consts::OK_STRING,
        manager::Manager,
        types::DbusErrorEnum,
        util::{engine_to_dbus_err_tuple, send_has_cache_signal},
    },
    engine::{Engine, EngineAction, Lockable, PoolIdentifier, PoolUuid},
    stratis::StratisError,
};

pub async fn init_cache_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    _counter: &Arc<AtomicU64>,
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
                true,
            ),
            conn_clone,
            man_clone,
            pool_uuid
        )
    })
    .await
    {
        Ok(Ok(action)) => {
            match action.changed() {
                Some(_) => {
                    match manager.read().await.pool_get_path(&pool_uuid) {
                        Some(p) => {
                            send_has_cache_signal(connection, p).await;
                        }
                        None => {
                            warn!("No object path associated with pool UUID {pool_uuid}; failed to send pool has cache change signals");
                        }
                    };
                    // TODO: Register blockdevs here.
                    (
                        // TODO: Change to blockdev object paths.
                        default_return,
                        DbusErrorEnum::OK as u16,
                        OK_STRING.to_string(),
                    )
                }
                None => {
                    (
                        // TODO: Change to blockdev object paths.
                        default_return,
                        DbusErrorEnum::OK as u16,
                        OK_STRING.to_string(),
                    )
                }
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

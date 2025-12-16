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
        types::DbusErrorEnum,
        util::{engine_to_dbus_err_tuple, send_pool_foreground_signals},
    },
    engine::{DevUuid, Engine, EngineAction, Lockable, PoolIdentifier, PoolUuid},
    stratis::StratisError,
};

pub async fn grow_physical_device_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
    dev: DevUuid,
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
            Some(_) => {
                if let Some(d) = diff {
                    send_pool_foreground_signals(connection, manager, pool_uuid, d).await;
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

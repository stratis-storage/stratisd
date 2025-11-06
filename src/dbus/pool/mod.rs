// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use tokio::sync::RwLock;
use zbus::{zvariant::ObjectPath, Connection};

use crate::{
    dbus::{consts, Manager},
    engine::{Engine, Lockable, PoolUuid},
    stratis::{StratisError, StratisResult},
};

mod pool_3_0;
mod pool_3_6;
mod pool_3_9;
mod shared;

pub use pool_3_9::PoolR9;

pub async fn register_pool<'a>(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    pool_uuid: PoolUuid,
) -> StratisResult<(ObjectPath<'a>, Vec<ObjectPath<'a>>)> {
    let path = ObjectPath::try_from(format!(
        "{}/{}",
        consts::STRATIS_BASE_PATH,
        counter.fetch_add(1, Ordering::AcqRel),
    ))?;
    PoolR9::register(
        engine,
        connection,
        manager,
        counter,
        path.clone(),
        pool_uuid,
    )
    .await?;

    manager.write().await.add_pool(&path, pool_uuid)?;

    Ok((path, Vec::default()))
}

pub async fn unregister_pool(
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    path: &ObjectPath<'_>,
) -> StratisResult<PoolUuid> {
    PoolR9::unregister(connection, path.clone()).await?;

    let mut lock = manager.write().await;
    let uuid = lock
        .pool_get_uuid(path)
        .ok_or_else(|| StratisError::Msg(format!("No UUID associated with path {path}")))?;
    lock.remove_pool(path);

    Ok(uuid)
}

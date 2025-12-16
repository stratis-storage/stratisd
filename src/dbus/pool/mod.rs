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
mod pool_3_1;
mod pool_3_2;
mod pool_3_6;
mod pool_3_9;
mod shared;

pub use pool_3_0::PoolR0;
pub use pool_3_1::PoolR1;
pub use pool_3_2::PoolR2;
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

    manager.write().await.add_pool(&path, pool_uuid)?;

    if let Err(e) = PoolR0::register(
        engine,
        connection,
        manager,
        counter,
        path.clone(),
        pool_uuid,
    )
    .await
    {
        warn!("Failed to register interface pool.r0 for pool with UUID {pool_uuid}: {e}");
    }
    if let Err(e) = PoolR1::register(
        engine,
        connection,
        manager,
        counter,
        path.clone(),
        pool_uuid,
    )
    .await
    {
        warn!("Failed to register interface pool.r1 for pool with UUID {pool_uuid}: {e}");
    }
    if let Err(e) = PoolR2::register(
        engine,
        connection,
        manager,
        counter,
        path.clone(),
        pool_uuid,
    )
    .await
    {
        warn!("Failed to register interface pool.r2 for pool with UUID {pool_uuid}: {e}");
    }
    if let Err(e) = PoolR9::register(
        engine,
        connection,
        manager,
        counter,
        path.clone(),
        pool_uuid,
    )
    .await
    {
        warn!("Failed to register interface pool.r9 for pool with UUID {pool_uuid}: {e}");
    }

    Ok((path, Vec::default()))
}

pub async fn unregister_pool(
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    path: &ObjectPath<'_>,
) -> StratisResult<PoolUuid> {
    let uuid = {
        let mut lock = manager.write().await;
        let uuid = lock
            .pool_get_uuid(path)
            .ok_or_else(|| StratisError::Msg(format!("No UUID associated with path {path}")))?;
        lock.remove_pool(path);
        uuid
    };

    if let Err(e) = PoolR0::unregister(connection, path.clone()).await {
        warn!("Failed to deregister interface pool.r0 for path {path}: {e}");
    }
    if let Err(e) = PoolR1::unregister(connection, path.clone()).await {
        warn!("Failed to deregister interface pool.r1 for path {path}: {e}");
    }
    if let Err(e) = PoolR2::unregister(connection, path.clone()).await {
        warn!("Failed to deregister interface pool.r2 for path {path}: {e}");
    }
    if let Err(e) = PoolR9::unregister(connection, path.clone()).await {
        warn!("Failed to deregister interface pool.r9 for path {path}: {e}");
    }

    Ok(uuid)
}

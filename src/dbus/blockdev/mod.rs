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
    engine::{DevUuid, Engine, Lockable, PoolUuid},
    stratis::{StratisError, StratisResult},
};

mod interface;
mod props;
mod shared;

pub use interface::BlockdevInterface;

pub async fn register_blockdev<'a>(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    pool_uuid: PoolUuid,
    dev_uuid: DevUuid,
) -> StratisResult<ObjectPath<'a>> {
    let path = ObjectPath::try_from(format!(
        "{}/{}",
        consts::STRATIS_BASE_PATH,
        counter.fetch_add(1, Ordering::AcqRel),
    ))?;
    if let Err(e) = BlockdevInterface::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        dev_uuid,
    )
    .await
    {
        warn!("Failed to register interface blockdev for pool with UUID {pool_uuid}: {e}");
    };
    manager.write().await.add_blockdev(&path, dev_uuid)?;
    Ok(path)
}

pub async fn unregister_blockdev(
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    path: &ObjectPath<'_>,
) -> StratisResult<DevUuid> {
    BlockdevInterface::unregister(connection, path.clone()).await?;

    let mut lock = manager.write().await;
    let uuid = lock
        .blockdev_get_uuid(path)
        .ok_or_else(|| StratisError::Msg(format!("No UUID associated with path {path}")))?;
    lock.remove_blockdev(path);

    Ok(uuid)
}

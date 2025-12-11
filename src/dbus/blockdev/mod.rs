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

mod blockdev_3_0;
mod blockdev_3_1;
mod blockdev_3_2;
mod blockdev_3_3;
mod blockdev_3_4;
mod blockdev_3_5;
mod blockdev_3_6;
mod blockdev_3_7;
mod blockdev_3_8;
mod blockdev_3_9;
mod shared;

pub use blockdev_3_0::BlockdevR0;
pub use blockdev_3_1::BlockdevR1;
pub use blockdev_3_2::BlockdevR2;
pub use blockdev_3_3::BlockdevR3;
pub use blockdev_3_4::BlockdevR4;
pub use blockdev_3_5::BlockdevR5;
pub use blockdev_3_6::BlockdevR6;
pub use blockdev_3_7::BlockdevR7;
pub use blockdev_3_8::BlockdevR8;
pub use blockdev_3_9::BlockdevR9;

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
    if let Err(e) = BlockdevR0::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        dev_uuid,
    )
    .await
    {
        warn!("Failed to register interface blockdev.r0 for pool with UUID {pool_uuid}: {e}");
    };
    if let Err(e) = BlockdevR1::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        dev_uuid,
    )
    .await
    {
        warn!("Failed to register interface blockdev.r1 for pool with UUID {pool_uuid}: {e}");
    };
    if let Err(e) = BlockdevR2::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        dev_uuid,
    )
    .await
    {
        warn!("Failed to register interface blockdev.r2 for pool with UUID {pool_uuid}: {e}");
    };
    if let Err(e) = BlockdevR3::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        dev_uuid,
    )
    .await
    {
        warn!("Failed to register interface blockdev.r3 for pool with UUID {pool_uuid}: {e}");
    };
    if let Err(e) = BlockdevR4::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        dev_uuid,
    )
    .await
    {
        warn!("Failed to register interface blockdev.r4 for pool with UUID {pool_uuid}: {e}");
    };
    if let Err(e) = BlockdevR5::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        dev_uuid,
    )
    .await
    {
        warn!("Failed to register interface blockdev.r5 for pool with UUID {pool_uuid}: {e}");
    };
    if let Err(e) = BlockdevR6::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        dev_uuid,
    )
    .await
    {
        warn!("Failed to register interface blockdev.r6 for pool with UUID {pool_uuid}: {e}");
    };
    if let Err(e) = BlockdevR7::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        dev_uuid,
    )
    .await
    {
        warn!("Failed to register interface blockdev.r7 for pool with UUID {pool_uuid}: {e}");
    };
    if let Err(e) = BlockdevR8::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        dev_uuid,
    )
    .await
    {
        warn!("Failed to register interface blockdev.r8 for pool with UUID {pool_uuid}: {e}");
    };
    if let Err(e) = BlockdevR9::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        dev_uuid,
    )
    .await
    {
        warn!("Failed to register interface blockdev.r9 for pool with UUID {pool_uuid}: {e}");
    };
    manager.write().await.add_blockdev(&path, dev_uuid)?;
    Ok(path)
}

#[allow(dead_code)]
// FIXME: should be used
pub async fn unregister_blockdev(
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    path: &ObjectPath<'_>,
) -> StratisResult<DevUuid> {
    BlockdevR0::unregister(connection, path.clone()).await?;
    BlockdevR1::unregister(connection, path.clone()).await?;
    BlockdevR2::unregister(connection, path.clone()).await?;
    BlockdevR3::unregister(connection, path.clone()).await?;
    BlockdevR4::unregister(connection, path.clone()).await?;
    BlockdevR5::unregister(connection, path.clone()).await?;
    BlockdevR6::unregister(connection, path.clone()).await?;
    BlockdevR7::unregister(connection, path.clone()).await?;
    BlockdevR8::unregister(connection, path.clone()).await?;
    BlockdevR9::unregister(connection, path.clone()).await?;

    let mut lock = manager.write().await;
    let uuid = lock
        .blockdev_get_uuid(path)
        .ok_or_else(|| StratisError::Msg(format!("No UUID associated with path {path}")))?;
    lock.remove_blockdev(path);

    Ok(uuid)
}

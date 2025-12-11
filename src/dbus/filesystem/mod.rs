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
    engine::{Engine, FilesystemUuid, Lockable, PoolUuid},
    stratis::{StratisError, StratisResult},
};

mod filesystem_3_0;
mod filesystem_3_1;
mod filesystem_3_2;
mod filesystem_3_3;
mod filesystem_3_4;
mod filesystem_3_5;
mod filesystem_3_6;
mod filesystem_3_7;
mod filesystem_3_8;
mod filesystem_3_9;
mod shared;

pub use filesystem_3_0::FilesystemR0;
pub use filesystem_3_1::FilesystemR1;
pub use filesystem_3_2::FilesystemR2;
pub use filesystem_3_3::FilesystemR3;
pub use filesystem_3_4::FilesystemR4;
pub use filesystem_3_5::FilesystemR5;
pub use filesystem_3_6::FilesystemR6;
pub use filesystem_3_7::FilesystemR7;
pub use filesystem_3_8::FilesystemR8;
pub use filesystem_3_9::FilesystemR9;

pub async fn register_filesystem<'a>(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    pool_uuid: PoolUuid,
    uuid: FilesystemUuid,
) -> StratisResult<ObjectPath<'a>> {
    let path = ObjectPath::try_from(format!(
        "{}/{}",
        consts::STRATIS_BASE_PATH,
        counter.fetch_add(1, Ordering::AcqRel),
    ))?;

    manager.write().await.add_filesystem(&path, uuid)?;

    FilesystemR0::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        uuid,
    )
    .await?;
    FilesystemR1::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        uuid,
    )
    .await?;
    FilesystemR2::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        uuid,
    )
    .await?;
    FilesystemR3::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        uuid,
    )
    .await?;
    FilesystemR4::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        uuid,
    )
    .await?;
    FilesystemR5::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        uuid,
    )
    .await?;
    FilesystemR6::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        uuid,
    )
    .await?;
    FilesystemR7::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        uuid,
    )
    .await?;
    FilesystemR8::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        uuid,
    )
    .await?;
    FilesystemR9::register(
        engine.clone(),
        connection,
        manager,
        path.clone(),
        pool_uuid,
        uuid,
    )
    .await?;

    Ok(path)
}

pub async fn unregister_filesystem(
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    path: &ObjectPath<'_>,
) -> StratisResult<FilesystemUuid> {
    let uuid = {
        let mut lock = manager.write().await;
        let uuid = lock
            .filesystem_get_uuid(path)
            .ok_or_else(|| StratisError::Msg(format!("No UUID associated with path {path}")))?;
        lock.remove_filesystem(path);
        uuid
    };

    FilesystemR0::unregister(connection, path.clone()).await?;
    FilesystemR1::unregister(connection, path.clone()).await?;
    FilesystemR2::unregister(connection, path.clone()).await?;
    FilesystemR3::unregister(connection, path.clone()).await?;
    FilesystemR4::unregister(connection, path.clone()).await?;
    FilesystemR5::unregister(connection, path.clone()).await?;
    FilesystemR6::unregister(connection, path.clone()).await?;
    FilesystemR7::unregister(connection, path.clone()).await?;
    FilesystemR8::unregister(connection, path.clone()).await?;
    FilesystemR9::unregister(connection, path.clone()).await?;

    Ok(uuid)
}

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
mod filesystem_3_9;
mod shared;

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

    FilesystemR9::register(engine, connection, path.clone(), pool_uuid, uuid).await?;

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

    FilesystemR9::unregister(connection, path.clone()).await?;

    Ok(uuid)
}

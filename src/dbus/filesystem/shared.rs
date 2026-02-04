// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{future::Future, sync::Arc};

use tokio::sync::RwLock;
use zbus::{fdo::Error, Connection};

use crate::{
    dbus::manager::Manager,
    engine::{
        Engine, Filesystem, FilesystemUuid, Lockable, Name, Pool, PoolIdentifier, PoolUuid,
        SomeLockWriteGuard,
    },
};

#[allow(clippy::too_many_arguments)]
pub async fn set_filesystem_prop<'a, 'b, V, F, Fut, S, SFut>(
    engine: &Arc<dyn Engine>,
    connection: &'a Arc<Connection>,
    manager: &'b Lockable<Arc<RwLock<Manager>>>,
    uuid: PoolUuid,
    fs_uuid: FilesystemUuid,
    value: V,
    f: F,
    send_signal: S,
) -> Result<(), zbus::Error>
where
    F: FnOnce(SomeLockWriteGuard<PoolUuid, dyn Pool>, FilesystemUuid, V) -> Fut,
    Fut: Future<Output = Result<bool, zbus::Error>>,
    S: FnOnce(&'a Arc<Connection>, &'b Lockable<Arc<RwLock<Manager>>>, FilesystemUuid) -> SFut,
    SFut: Future<Output = ()>,
{
    let guard = engine
        .get_mut_pool(PoolIdentifier::Uuid(uuid))
        .await
        .ok_or_else(|| zbus::Error::Failure(format!("No pool associated with UUID {uuid}")))?;

    let changed = f(guard, fs_uuid, value).await?;
    // Guard is dropped here, lock is released

    if changed {
        send_signal(connection, manager, fs_uuid).await;
    }

    Ok(())
}

pub async fn filesystem_prop<R>(
    engine: &Arc<dyn Engine>,
    uuid: PoolUuid,
    fs_uuid: FilesystemUuid,
    f: impl Fn(Name, Name, FilesystemUuid, &dyn Filesystem) -> R,
) -> Result<R, Error> {
    let guard = engine
        .get_pool(PoolIdentifier::Uuid(uuid))
        .await
        .ok_or_else(|| Error::Failed(format!("No pool associated with UUID {uuid}")))?;

    let pool_name = guard.as_tuple().0;

    let (name, fs) = guard
        .get_filesystem(fs_uuid)
        .ok_or_else(|| Error::Failed(format!("No filesystem associated with UUID {fs_uuid}")))?;

    Ok(f(pool_name, name, fs_uuid, fs))
}

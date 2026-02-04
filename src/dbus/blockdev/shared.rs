// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{future::Future, sync::Arc};

use tokio::sync::RwLock;
use zbus::{self, fdo::Error, Connection};

use crate::{
    dbus::manager::Manager,
    engine::{
        BlockDev, BlockDevTier, DevUuid, Engine, Lockable, Pool, PoolIdentifier, PoolUuid,
        SomeLockWriteGuard,
    },
};

#[allow(clippy::too_many_arguments)]
pub async fn set_blockdev_prop<'a, 'b, V, F, Fut, S, SFut>(
    engine: &Arc<dyn Engine>,
    connection: &'a Arc<Connection>,
    manager: &'b Lockable<Arc<RwLock<Manager>>>,
    uuid: PoolUuid,
    bd_uuid: DevUuid,
    value: V,
    f: F,
    send_signal: S,
) -> Result<(), zbus::Error>
where
    F: FnOnce(SomeLockWriteGuard<PoolUuid, dyn Pool>, DevUuid, V) -> Fut,
    Fut: Future<Output = Result<bool, zbus::Error>>,
    S: FnOnce(&'a Arc<Connection>, &'b Lockable<Arc<RwLock<Manager>>>, DevUuid) -> SFut,
    SFut: Future<Output = ()>,
{
    let guard = engine
        .get_mut_pool(PoolIdentifier::Uuid(uuid))
        .await
        .ok_or_else(|| zbus::Error::Failure(format!("No pool associated with UUID {uuid}")))?;

    let changed = f(guard, bd_uuid, value).await?;
    // Guard is dropped here, lock is released

    if changed {
        send_signal(connection, manager, bd_uuid).await;
    }

    Ok(())
}

pub async fn blockdev_prop<R>(
    engine: &Arc<dyn Engine>,
    uuid: PoolUuid,
    bd_uuid: DevUuid,
    f: impl Fn(BlockDevTier, DevUuid, &dyn BlockDev) -> R,
) -> Result<R, Error> {
    let guard = engine
        .get_pool(PoolIdentifier::Uuid(uuid))
        .await
        .ok_or_else(|| Error::Failed(format!("No pool associated with UUID {uuid}")))?;

    let (tier, bd) = guard
        .get_blockdev(bd_uuid)
        .ok_or_else(|| Error::Failed(format!("No block device associated with UUID {bd_uuid}")))?;

    Ok(f(tier, bd_uuid, bd))
}

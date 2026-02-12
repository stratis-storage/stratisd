// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{future::Future, sync::Arc};

use tokio::sync::RwLock;
use zbus::{fdo::Error, Connection};

use crate::{
    dbus::manager::Manager,
    engine::{
        Engine, Lockable, Pool, PoolIdentifier, PoolUuid, SomeLockReadGuard, SomeLockWriteGuard,
    },
};

async fn get_pool(
    engine: &Arc<dyn Engine>,
    uuid: PoolUuid,
) -> Result<SomeLockReadGuard<PoolUuid, dyn Pool>, Error> {
    engine
        .get_pool(PoolIdentifier::Uuid(uuid))
        .await
        .ok_or_else(|| Error::Failed(format!("No pool associated with UUID {uuid}")))
}

async fn get_pool_mut(
    engine: &Arc<dyn Engine>,
    uuid: PoolUuid,
) -> Result<SomeLockWriteGuard<PoolUuid, dyn Pool>, zbus::Error> {
    engine
        .get_mut_pool(PoolIdentifier::Uuid(uuid))
        .await
        .ok_or_else(|| zbus::Error::Failure(format!("No pool associated with UUID {uuid}")))
}

pub async fn pool_prop<R>(
    engine: &Arc<dyn Engine>,
    uuid: PoolUuid,
    f: impl Fn(SomeLockReadGuard<PoolUuid, dyn Pool>) -> R,
) -> Result<R, Error> {
    let guard = get_pool(engine, uuid).await?;

    Ok(f(guard))
}

pub async fn try_pool_prop<R>(
    engine: &Arc<dyn Engine>,
    uuid: PoolUuid,
    f: impl Fn(SomeLockReadGuard<PoolUuid, dyn Pool>) -> R,
) -> Result<R, Error> {
    let guard = get_pool(engine, uuid).await?;

    Ok(f(guard))
}

pub async fn set_pool_prop<'a, 'b, I, F, Fut, S, SFut>(
    engine: &Arc<dyn Engine>,
    connection: &'a Arc<Connection>,
    manager: &'b Lockable<Arc<RwLock<Manager>>>,
    uuid: PoolUuid,
    f: F,
    input: I,
    send_signal: S,
) -> Result<(), zbus::Error>
where
    F: FnOnce(SomeLockWriteGuard<PoolUuid, dyn Pool>, PoolUuid, I) -> Fut,
    Fut: Future<Output = Result<bool, zbus::Error>>,
    S: FnOnce(&'a Arc<Connection>, &'b Lockable<Arc<RwLock<Manager>>>, PoolUuid) -> SFut,
    SFut: Future<Output = ()>,
{
    let guard = get_pool_mut(engine, uuid).await?;

    let changed = f(guard, uuid, input).await?;
    // Guard is dropped here, lock is released

    if changed {
        send_signal(connection, manager, uuid).await;
    }

    Ok(())
}

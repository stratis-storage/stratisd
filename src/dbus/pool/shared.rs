// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use zbus::fdo::Error;

use crate::engine::{Engine, Pool, PoolIdentifier, PoolUuid, SomeLockReadGuard};

async fn get_pool(
    engine: &Arc<dyn Engine>,
    uuid: PoolUuid,
) -> Result<SomeLockReadGuard<PoolUuid, dyn Pool>, Error> {
    engine
        .get_pool(PoolIdentifier::Uuid(uuid))
        .await
        .ok_or_else(|| Error::Failed(format!("No pool associated with UUID {uuid}")))
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

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use zbus::{self, fdo::Error};

use crate::engine::{
    BlockDev, BlockDevTier, DevUuid, Engine, Pool, PoolIdentifier, PoolUuid, SomeLockWriteGuard,
};

pub async fn set_blockdev_prop<V>(
    engine: &Arc<dyn Engine>,
    uuid: PoolUuid,
    bd_uuid: DevUuid,
    value: V,
    f: impl Fn(&mut SomeLockWriteGuard<PoolUuid, dyn Pool>, DevUuid, V) -> Result<(), zbus::Error>,
) -> Result<(), zbus::Error> {
    let mut guard = engine
        .get_mut_pool(PoolIdentifier::Uuid(uuid))
        .await
        .ok_or_else(|| zbus::Error::Failure(format!("No pool associated with UUID {uuid}")))?;

    f(&mut guard, bd_uuid, value)
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

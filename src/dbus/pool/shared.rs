// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use zbus::{fdo::Error, zvariant::OwnedValue};

use crate::engine::{Engine, Pool, PoolIdentifier, PoolUuid, SomeLockReadGuard};

pub async fn pool_prop<R>(
    engine: &Arc<dyn Engine>,
    uuid: PoolUuid,
    f: impl Fn(SomeLockReadGuard<PoolUuid, dyn Pool>) -> R,
) -> Result<OwnedValue, Error>
where
    OwnedValue: From<R>,
{
    let guard = engine
        .get_pool(PoolIdentifier::Uuid(uuid))
        .await
        .ok_or_else(|| Error::Failed(format!("No pool associated with UUID {uuid}")))?;

    Ok(OwnedValue::from(f(guard)))
}

pub async fn try_pool_prop<R>(
    engine: &Arc<dyn Engine>,
    uuid: PoolUuid,
    f: impl Fn(SomeLockReadGuard<PoolUuid, dyn Pool>) -> R,
) -> Result<OwnedValue, Error>
where
    OwnedValue: TryFrom<R>,
{
    let guard = engine
        .get_pool(PoolIdentifier::Uuid(uuid))
        .await
        .ok_or_else(|| Error::Failed(format!("No pool associated with UUID {uuid}")))?;

    OwnedValue::try_from(f(guard))
        .map_err(|_| Error::Failed("D-Bus data type conversion failed".to_string()))
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use zbus::fdo::Error;

use crate::engine::{
    Engine, Filesystem, FilesystemUuid, Name, Pool, PoolIdentifier, PoolUuid, SomeLockWriteGuard,
};

pub async fn set_filesystem_prop<V>(
    engine: &Arc<dyn Engine>,
    uuid: PoolUuid,
    fs_uuid: FilesystemUuid,
    value: V,
    f: impl Fn(
        &mut SomeLockWriteGuard<PoolUuid, dyn Pool>,
        FilesystemUuid,
        V,
    ) -> Result<(), zbus::Error>,
) -> Result<(), zbus::Error> {
    let mut guard = engine
        .get_mut_pool(PoolIdentifier::Uuid(uuid))
        .await
        .ok_or_else(|| zbus::Error::Failure(format!("No pool associated with UUID {uuid}")))?;

    f(&mut guard, fs_uuid, value)
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

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use zbus::fdo::Error;

use crate::engine::{Engine, Filesystem, FilesystemUuid, Name, PoolIdentifier, PoolUuid};

pub async fn filesystem_prop<R>(
    engine: &Arc<dyn Engine>,
    uuid: PoolUuid,
    fs_uuid: FilesystemUuid,
    f: impl Fn(Name, FilesystemUuid, &dyn Filesystem) -> R,
) -> Result<R, Error> {
    let guard = engine
        .get_pool(PoolIdentifier::Uuid(uuid))
        .await
        .ok_or_else(|| Error::Failed(format!("No pool associated with UUID {uuid}")))?;

    let (name, fs) = guard
        .get_filesystem(fs_uuid)
        .ok_or_else(|| Error::Failed(format!("No filesystem associated with UUID {fs_uuid}")))?;

    Ok(f(name, fs_uuid, fs))
}

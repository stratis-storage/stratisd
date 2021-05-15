// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::os::unix::io::RawFd;

use crate::{
    engine::{KeyDescription, LockableEngine, MappingCreateAction, MappingDeleteAction, PoolUuid},
    stratis::StratisResult,
};

// stratis-min key set
pub async fn key_set(
    engine: LockableEngine,
    key_desc: &KeyDescription,
    key_fd: RawFd,
) -> StratisResult<Option<bool>> {
    Ok(
        match engine
            .lock()
            .await
            .get_key_handler_mut()
            .set(key_desc, key_fd)?
        {
            MappingCreateAction::Created(_) => Some(false),
            MappingCreateAction::ValueChanged(_) => Some(true),
            MappingCreateAction::Identity => None,
        },
    )
}

// stratis-min key unset
pub async fn key_unset(engine: LockableEngine, key_desc: &KeyDescription) -> StratisResult<bool> {
    Ok(
        match engine.lock().await.get_key_handler_mut().unset(key_desc)? {
            MappingDeleteAction::Deleted(_) => true,
            MappingDeleteAction::Identity => false,
        },
    )
}

// stratis-min key [list]
pub async fn key_list(engine: LockableEngine) -> StratisResult<Vec<KeyDescription>> {
    Ok(engine
        .lock()
        .await
        .get_key_handler_mut()
        .list()?
        .into_iter()
        .collect())
}

pub async fn key_get_desc(engine: LockableEngine, pool_uuid: PoolUuid) -> Option<KeyDescription> {
    let locked_pools = engine.lock().await.locked_pools();
    locked_pools
        .get(&pool_uuid)
        .and_then(|info| info.info.key_description.to_owned())
}

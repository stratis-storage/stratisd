// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::os::unix::io::RawFd;

use crate::{
    engine::{
        Engine, KeyActions, KeyDescription, LockableEngine, MappingCreateAction,
        MappingDeleteAction, PoolUuid,
    },
    stratis::StratisResult,
};

// stratis-min key set
pub async fn key_set<E>(
    engine: LockableEngine<E>,
    key_desc: &KeyDescription,
    key_fd: RawFd,
) -> StratisResult<Option<bool>>
where
    E: Engine,
{
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
pub async fn key_unset<E>(
    engine: LockableEngine<E>,
    key_desc: &KeyDescription,
) -> StratisResult<bool>
where
    E: Engine,
{
    Ok(
        match engine.lock().await.get_key_handler_mut().unset(key_desc)? {
            MappingDeleteAction::Deleted(_) => true,
            MappingDeleteAction::Identity => false,
        },
    )
}

// stratis-min key [list]
pub async fn key_list<E>(engine: LockableEngine<E>) -> StratisResult<Vec<KeyDescription>>
where
    E: Engine,
{
    Ok(engine
        .lock()
        .await
        .get_key_handler_mut()
        .list()?
        .into_iter()
        .collect())
}

pub async fn key_get_desc<E>(
    engine: LockableEngine<E>,
    pool_uuid: PoolUuid,
) -> StratisResult<Option<KeyDescription>>
where
    E: Engine,
{
    let locked_pools = engine.lock().await.locked_pools();
    match locked_pools.get(&pool_uuid) {
        Some(info) => Ok(info.info.key_description()?.cloned()),
        None => Ok(None),
    }
}

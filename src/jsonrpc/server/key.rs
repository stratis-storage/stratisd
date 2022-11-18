// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{os::unix::io::RawFd, sync::Arc};

use crate::{
    engine::{
        Engine, KeyActions, KeyDescription, MappingCreateAction, MappingDeleteAction, Pool,
        PoolIdentifier, PoolUuid,
    },
    stratis::{StratisError, StratisResult},
};

// stratis-min key set
pub async fn key_set<E>(
    engine: Arc<E>,
    key_desc: &KeyDescription,
    key_fd: RawFd,
) -> StratisResult<Option<bool>>
where
    E: Engine,
{
    Ok(
        match engine.get_key_handler_mut().await.set(key_desc, key_fd)? {
            MappingCreateAction::Created(_) => Some(false),
            MappingCreateAction::ValueChanged(_) => Some(true),
            MappingCreateAction::Identity => None,
        },
    )
}

// stratis-min key unset
pub async fn key_unset<E>(engine: Arc<E>, key_desc: &KeyDescription) -> StratisResult<bool>
where
    E: Engine,
{
    Ok(match engine.get_key_handler_mut().await.unset(key_desc)? {
        MappingDeleteAction::Deleted(_) => true,
        MappingDeleteAction::Identity => false,
    })
}

// stratis-min key [list]
pub async fn key_list<E>(engine: Arc<E>) -> StratisResult<Vec<KeyDescription>>
where
    E: Engine,
{
    Ok(engine
        .get_key_handler_mut()
        .await
        .list()?
        .into_iter()
        .collect())
}

pub async fn key_get_desc<E>(
    engine: Arc<E>,
    id: PoolIdentifier<PoolUuid>,
) -> StratisResult<Option<KeyDescription>>
where
    E: Engine,
{
    let locked = engine.locked_pools().await;
    let guard = engine.get_pool(id.clone()).await;
    if let Some((_, _, pool)) = guard.as_ref().map(|guard| guard.as_tuple()) {
        match pool.encryption_info() {
            Some(ei) => ei.key_description().map(|opt| opt.cloned()),
            None => Ok(None),
        }
    } else if let Some(info) = locked.locked.get(match id {
        PoolIdentifier::Uuid(ref u) => u,
        PoolIdentifier::Name(ref n) => locked
            .name_to_uuid
            .get(n)
            .ok_or_else(|| StratisError::Msg(format!("Pool with name {} not found", n)))?,
    }) {
        info.info.key_description().map(|opt| opt.cloned())
    } else {
        Err(StratisError::Msg(format!("Pool with {} not found", id)))
    }
}

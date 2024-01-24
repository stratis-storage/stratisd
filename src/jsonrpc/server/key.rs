// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{os::unix::io::RawFd, sync::Arc};

use crate::{
    engine::{
        Engine, KeyDescription, MappingCreateAction, MappingDeleteAction, PoolIdentifier, PoolUuid,
    },
    stratis::{StratisError, StratisResult},
};

// stratis-min key set
pub async fn key_set(
    engine: Arc<dyn Engine>,
    key_desc: &KeyDescription,
    key_fd: RawFd,
) -> StratisResult<Option<bool>> {
    Ok(
        match engine.get_key_handler().await.set(key_desc, key_fd)? {
            MappingCreateAction::Created(_) => Some(false),
            MappingCreateAction::ValueChanged(_) => Some(true),
            MappingCreateAction::Identity => None,
        },
    )
}

// stratis-min key unset
pub async fn key_unset(engine: Arc<dyn Engine>, key_desc: &KeyDescription) -> StratisResult<bool> {
    Ok(match engine.get_key_handler().await.unset(key_desc)? {
        MappingDeleteAction::Deleted(_) => true,
        MappingDeleteAction::Identity => false,
    })
}

// stratis-min key [list]
pub async fn key_list(engine: Arc<dyn Engine>) -> StratisResult<Vec<KeyDescription>> {
    Ok(engine.get_key_handler().await.list()?.into_iter().collect())
}

pub async fn key_get_desc(
    engine: Arc<dyn Engine>,
    id: PoolIdentifier<PoolUuid>,
) -> StratisResult<Option<KeyDescription>> {
    let stopped = engine.stopped_pools().await;
    let guard = engine.get_pool(id.clone()).await;
    if let Some((_, _, pool)) = guard.as_ref().map(|guard| guard.as_tuple()) {
        match pool.encryption_info() {
            Some(ei) => ei.key_description().map(|opt| opt.cloned()),
            None => Ok(None),
        }
    } else if let Some(info) = stopped.stopped.get(match id {
        PoolIdentifier::Uuid(ref u) => u,
        PoolIdentifier::Name(ref n) => stopped
            .name_to_uuid
            .get(n)
            .ok_or_else(|| StratisError::Msg(format!("Pool with name {n} not found")))?,
    }) {
        if let Some(ref i) = info.info {
            i.key_description().map(|opt| opt.cloned())
        } else {
            Ok(None)
        }
    } else {
        Err(StratisError::Msg(format!("Pool with {id} not found")))
    }
}

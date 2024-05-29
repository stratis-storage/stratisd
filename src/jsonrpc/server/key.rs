// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{os::unix::io::RawFd, sync::Arc};

use crate::{
    engine::{Engine, KeyDescription, MappingCreateAction, MappingDeleteAction},
    stratis::StratisResult,
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

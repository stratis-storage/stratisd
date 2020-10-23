// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{os::unix::io::RawFd, sync::Arc};

use tokio::sync::Mutex;

use crate::{
    engine::{DeleteAction, Engine, KeyDescription, MappingCreateAction, PoolUuid},
    stratis::StratisResult,
};

/// This method sets a key in the kernel keyring. It accepts an optional keyfile path
/// and if this is not provided, the user is prompted for a passphrase. When `no_tty`
/// is true, this command assumes that no TTY is available for setting terminal
/// settings and settings such as `NOECHO` are not set. This option should be
/// used carefully as it will cause the password to be echoed on the screen if
/// invoked interactively.
// stratis-min key set
pub async fn key_set(
    engine: Arc<Mutex<dyn Engine>>,
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

pub async fn key_unset(
    engine: Arc<Mutex<dyn Engine>>,
    key_desc: &KeyDescription,
) -> StratisResult<bool> {
    Ok(
        match engine.lock().await.get_key_handler_mut().unset(key_desc)? {
            DeleteAction::Deleted(_) => true,
            DeleteAction::Identity => false,
        },
    )
}

pub async fn key_list(engine: Arc<Mutex<dyn Engine>>) -> StratisResult<Vec<KeyDescription>> {
    Ok(engine
        .lock()
        .await
        .get_key_handler_mut()
        .list()?
        .into_iter()
        .collect())
}

pub async fn key_get_desc(
    engine: Arc<Mutex<dyn Engine>>,
    pool_uuid: PoolUuid,
) -> Option<KeyDescription> {
    let locked_pools = engine.lock().await.locked_pools();
    locked_pools
        .get(&pool_uuid)
        .map(|info| info.key_description.to_owned())
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::os::unix::io::RawFd;

use crate::{
    engine::{DeleteAction, Engine, KeyDescription, MappingCreateAction, PoolUuid, StratEngine},
    stratis::StratisResult,
};

/// This method sets a key in the kernel keyring. It accepts an optional keyfile path
/// and if this is not provided, the user is prompted for a passphrase. When `no_tty`
/// is true, this command assumes that no TTY is available for setting terminal
/// settings and settings such as `NOECHO` are not set. This option should be
/// used carefully as it will cause the password to be echoed on the screen if
/// invoked interactively.
// stratis-min key set
pub fn key_set(
    engine: &mut StratEngine,
    key_desc: &KeyDescription,
    key_fd: RawFd,
    interactive: Option<bool>,
) -> StratisResult<Option<bool>> {
    Ok(
        match engine
            .get_key_handler_mut()
            .set(key_desc, key_fd, interactive)?
        {
            MappingCreateAction::Created(()) => Some(false),
            MappingCreateAction::ValueChanged(()) => Some(true),
            MappingCreateAction::Identity => None,
        },
    )
}

pub fn key_unset(engine: &mut StratEngine, key_desc: &KeyDescription) -> StratisResult<bool> {
    Ok(match engine.get_key_handler_mut().unset(key_desc)? {
        DeleteAction::Deleted(()) => true,
        DeleteAction::Identity => false,
    })
}

pub fn key_list(engine: &mut StratEngine) -> StratisResult<Vec<KeyDescription>> {
    Ok(engine.get_key_handler_mut().list()?.into_iter().collect())
}

pub fn key_get_desc(engine: &mut StratEngine, pool_uuid: PoolUuid) -> Option<KeyDescription> {
    let locked_pools = engine.locked_pools();
    locked_pools.get(&pool_uuid).map(|kd| kd.to_owned())
}

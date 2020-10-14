// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::os::unix::io::RawFd;

use crate::{
    engine::{DeleteAction, Engine, MappingCreateAction, PoolUuid, StratEngine},
    jsonrpc::{
        consts::{OP_OK, OP_OK_STR},
        utils::stratis_error_to_return,
    },
    stratis::StratisResult,
};

pub fn key_set_internal(
    engine: &mut StratEngine,
    key_desc: String,
    key_fd: RawFd,
    interactive: Option<bool>,
) -> StratisResult<MappingCreateAction<()>> {
    engine
        .get_key_handler_mut()
        .set(key_desc.as_str(), key_fd, interactive)
}

/// This method sets a key in the kernel keyring. It accepts an optional keyfile path
/// and if this is not provided, the user is prompted for a passphrase. When `no_tty`
/// is true, this command assumes that no TTY is available for setting terminal
/// settings and settings such as `NOECHO` are not set. This option should be
/// used carefully as it will cause the password to be echoed on the screen if
/// invoked interactively.
// stratis-min key set
pub fn key_set(
    engine: &mut StratEngine,
    key_desc: String,
    key_fd: RawFd,
    interactive: Option<bool>,
) -> (Option<bool>, u16, String) {
    let default_return = None;

    let val = match key_set_internal(engine, key_desc, key_fd, interactive) {
        Ok(MappingCreateAction::Created(())) => Some(false),
        Ok(MappingCreateAction::ValueChanged(())) => Some(true),
        Ok(MappingCreateAction::Identity) => default_return,
        Err(e) => {
            let (rc, rs) = stratis_error_to_return(e);
            return (default_return, rc, rs);
        }
    };
    (val, OP_OK, OP_OK_STR.to_string())
}

pub fn key_unset(engine: &mut StratEngine, key_desc: String) -> (bool, u16, String) {
    let default_return = false;

    let val = match engine.get_key_handler_mut().unset(key_desc.as_str()) {
        Ok(DeleteAction::Deleted(())) => true,
        Ok(DeleteAction::Identity) => false,
        Err(e) => {
            let (rc, rs) = stratis_error_to_return(e);
            return (default_return, rc, rs);
        }
    };
    (val, OP_OK, OP_OK_STR.to_string())
}

pub fn key_list(engine: &mut StratEngine) -> (Vec<String>, u16, String) {
    let val = match engine.get_key_handler_mut().list() {
        Ok(list) => list
            .into_iter()
            .map(|kd| kd.as_application_str().to_string())
            .collect(),
        Err(e) => {
            let (rc, rs) = stratis_error_to_return(e);
            return (Vec::new(), rc, rs);
        }
    };
    (val, OP_OK, OP_OK_STR.to_string())
}

pub fn key_get_desc(engine: &mut StratEngine, pool_uuid: PoolUuid) -> Option<String> {
    let locked_pools = engine.locked_pools();
    locked_pools
        .get(&pool_uuid)
        .map(|kd| kd.as_application_str().to_owned())
}

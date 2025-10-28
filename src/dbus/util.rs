// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, sync::Arc};

use zbus::Connection;

use devicemapper::DmError;

use crate::{
    dbus::types::DbusErrorEnum,
    engine::{FilesystemUuid, PoolDiff, PoolUuid, StratFilesystemDiff},
    stratis::{StratisError, StratisResult},
};

/// Convert a tuple as option to an Option type
pub fn tuple_to_option<T>(value: (bool, T)) -> Option<T> {
    if value.0 {
        Some(value.1)
    } else {
        None
    }
}

/// Translates an engine error to the (errorcode, string) tuple that Stratis
/// D-Bus methods return.
pub fn engine_to_dbus_err_tuple(err: &StratisError) -> (u16, String) {
    let description = match *err {
        StratisError::DM(DmError::Core(ref err)) => err.to_string(),
        ref err => err.to_string(),
    };
    (DbusErrorEnum::ERROR as u16, description)
}

#[allow(clippy::implicit_hasher)]
pub fn send_pool_background_signals(
    _connection: &Arc<Connection>,
    _diffs: HashMap<PoolUuid, PoolDiff>,
) -> StratisResult<()> {
    Ok(())
}

#[allow(clippy::implicit_hasher)]
pub fn send_fs_background_signals(
    _connection: &Arc<Connection>,
    _diffs: HashMap<FilesystemUuid, StratFilesystemDiff>,
) -> StratisResult<()> {
    Ok(())
}

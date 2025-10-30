// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, sync::Arc};

use tokio::sync::RwLock;
use zbus::Connection;

use devicemapper::DmError;

use crate::{
    dbus::{manager::Manager, pool::PoolR9, types::DbusErrorEnum},
    engine::{FilesystemUuid, Lockable, PoolDiff, PoolUuid, StratFilesystemDiff},
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

/// Convert an option type to a tuple as option
pub fn option_to_tuple<T>(value: Option<T>, default: T) -> (bool, T) {
    match value {
        Some(v) => (true, v),
        None => (false, default),
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
pub async fn send_pool_background_signals(
    manager: Lockable<Arc<RwLock<Manager>>>,
    connection: &Arc<Connection>,
    diffs: HashMap<PoolUuid, PoolDiff>,
) -> StratisResult<()> {
    let dbus = manager.read().await;
    for (uuid, diff) in diffs {
        if diff.thin_pool.allocated_size.changed().is_some() {
            let iface_ref = connection
                .object_server()
                .interface::<_, PoolR9>(dbus.pools.get(&uuid).ok_or_else(|| {
                    StratisError::Msg(format!("No pool associated with UUID {uuid}"))
                })?)
                .await?;
            iface_ref
                .get_mut()
                .await
                .allocated_size_changed(iface_ref.signal_emitter())
                .await?;
        }
        if diff.thin_pool.used.changed().is_some() {
            let iface_ref = connection
                .object_server()
                .interface::<_, PoolR9>(dbus.pools.get(&uuid).ok_or_else(|| {
                    StratisError::Msg(format!("No pool associated with UUID {uuid}"))
                })?)
                .await?;
            iface_ref
                .get_mut()
                .await
                .total_physical_used_changed(iface_ref.signal_emitter())
                .await?;
        }
    }

    Ok(())
}

#[allow(clippy::implicit_hasher)]
pub async fn send_fs_background_signals(
    _manager: Lockable<Arc<RwLock<Manager>>>,
    _connection: &Arc<Connection>,
    _diffs: HashMap<FilesystemUuid, StratFilesystemDiff>,
) -> StratisResult<()> {
    Ok(())
}

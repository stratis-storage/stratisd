// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, fmt::Display, sync::Arc};

use tokio::sync::RwLock;
use zbus::{
    zvariant::{ObjectPath, Value},
    Connection,
};

use devicemapper::DmError;

use crate::{
    dbus::{
        consts::STRATIS_BASE_PATH,
        manager::{Manager, ManagerR0},
        pool::PoolR9,
        types::DbusErrorEnum,
    },
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

/// Map a result containing an option obtained for the FetchProperties interface to
/// a value used to represent both the result and option.  An error in the result
/// argument yields a false in the return value, indicating that the value
/// returned is a string representation of the error encountered in
/// obtaining the value, and not the value requested. If the first boolean is true,
/// the variant will be a tuple of type (bool, T). If the second boolean if false,
/// this indicates None. If it is true, the value for T is the Some(_) value.
pub fn result_option_to_tuple<'a, T, E>(
    result: Result<Option<T>, E>,
    default: T,
) -> (bool, Value<'a>)
where
    E: Display,
    Value<'a>: From<T> + From<(bool, T)>,
{
    let (success, value) = match result {
        Ok(value) => (true, Value::from(option_to_tuple(value, default))),
        Err(e) => (false, Value::from(e.to_string())),
    };
    (success, value)
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
                .interface::<_, PoolR9>(dbus.pool_get_path(&uuid).ok_or_else(|| {
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
                .interface::<_, PoolR9>(dbus.pool_get_path(&uuid).ok_or_else(|| {
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

pub async fn send_locked_pools_signals(connection: &Arc<Connection>) -> StratisResult<()> {
    let path = ObjectPath::from_static_str(STRATIS_BASE_PATH)?;

    let iface_ref = connection
        .object_server()
        .interface::<_, ManagerR0>(path)
        .await?;
    let mut_iface_ref = iface_ref.get_mut().await;
    mut_iface_ref
        .locked_pools_changed(iface_ref.signal_emitter())
        .await?;

    Ok(())
}

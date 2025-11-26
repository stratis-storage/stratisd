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
        manager::{
            Manager, ManagerR0, ManagerR1, ManagerR2, ManagerR3, ManagerR4, ManagerR5, ManagerR6,
            ManagerR7, ManagerR8, ManagerR9,
        },
        pool::PoolR9,
        types::DbusErrorEnum,
    },
    engine::{FilesystemUuid, Lockable, PoolDiff, PoolUuid, StratFilesystemDiff},
    stratis::StratisError,
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
) {
    let dbus = manager.read().await;
    for (uuid, diff) in diffs {
        if diff.thin_pool.allocated_size.changed().is_some() {
            let pool_path = match dbus.pool_get_path(&uuid) {
                Some(path) => path,
                None => {
                    warn!("No pool associated with UUID {uuid}, skipping allocated_size_changed signal");
                    continue;
                }
            };
            match connection
                .object_server()
                .interface::<_, PoolR9>(pool_path)
                .await
            {
                Ok(iface_ref) => {
                    if let Err(e) = iface_ref
                        .get_mut()
                        .await
                        .allocated_size_changed(iface_ref.signal_emitter())
                        .await
                    {
                        warn!("Failed to send allocated_size_changed signal for pool {uuid}: {e}");
                    }
                }
                Err(e) => {
                    warn!("Failed to get interface for pool {uuid} to send allocated_size_changed signal: {e}");
                }
            }
        }
        if diff.thin_pool.used.changed().is_some() {
            let pool_path = match dbus.pool_get_path(&uuid) {
                Some(path) => path,
                None => {
                    warn!("No pool associated with UUID {uuid}, skipping total_physical_used_changed signal");
                    continue;
                }
            };
            match connection
                .object_server()
                .interface::<_, PoolR9>(pool_path)
                .await
            {
                Ok(iface_ref) => {
                    if let Err(e) = iface_ref
                        .get_mut()
                        .await
                        .total_physical_used_changed(iface_ref.signal_emitter())
                        .await
                    {
                        warn!("Failed to send total_physical_used_changed signal for pool {uuid}: {e}");
                    }
                }
                Err(e) => {
                    warn!("Failed to get interface for pool {uuid} to send total_physical_used_changed signal: {e}");
                }
            }
        }
    }
}

#[allow(clippy::implicit_hasher)]
pub async fn send_fs_background_signals(
    _manager: Lockable<Arc<RwLock<Manager>>>,
    _connection: &Arc<Connection>,
    _diffs: HashMap<FilesystemUuid, StratFilesystemDiff>,
) {
}

pub async fn send_locked_pools_signals(connection: &Arc<Connection>) {
    let path = match ObjectPath::from_static_str(STRATIS_BASE_PATH) {
        Ok(path) => path,
        Err(e) => {
            warn!("Failed to convert string to object path: {e}");
            return;
        }
    };

    match connection
        .object_server()
        .interface::<_, ManagerR0>(&path)
        .await
    {
        Ok(iface_ref) => {
            let mut_iface_ref = iface_ref.get_mut().await;
            if let Err(e) = mut_iface_ref
                .locked_pools_changed(iface_ref.signal_emitter())
                .await
            {
                warn!("Failed to send property changed signal for locked pools on interface Manager.r0: {e}");
            }
        }
        Err(e) => {
            warn!("Failed to get Manager.r0 interface: {e}");
        }
    }

    match connection
        .object_server()
        .interface::<_, ManagerR1>(&path)
        .await
    {
        Ok(iface_ref) => {
            let mut_iface_ref = iface_ref.get_mut().await;
            if let Err(e) = mut_iface_ref
                .locked_pools_changed(iface_ref.signal_emitter())
                .await
            {
                warn!("Failed to send property changed signal for locked pools on interface Manager.r1: {e}");
            }
        }
        Err(e) => {
            warn!("Failed to get Manager.r1 interface: {e}");
        }
    }
}

pub async fn send_stopped_pools_signals(connection: &Arc<Connection>) {
    let path = match ObjectPath::from_static_str(STRATIS_BASE_PATH) {
        Ok(path) => path,
        Err(e) => {
            warn!("Failed to create object path for stopped_pools_changed signal: {e}");
            return;
        }
    };

    match connection
        .object_server()
        .interface::<_, ManagerR2>(&path)
        .await
    {
        Ok(iface_ref) => {
            let mut_iface_ref = iface_ref.get_mut().await;
            if let Err(e) = mut_iface_ref
                .stopped_pools_changed(iface_ref.signal_emitter())
                .await
            {
                warn!("Failed to send property changed signal for stopped pools on interface Manager.r2: {e}");
            }
        }
        Err(e) => {
            warn!("Failed to get Manager.r2 interface: {e}");
        }
    }

    match connection
        .object_server()
        .interface::<_, ManagerR3>(&path)
        .await
    {
        Ok(iface_ref) => {
            let mut_iface_ref = iface_ref.get_mut().await;
            if let Err(e) = mut_iface_ref
                .stopped_pools_changed(iface_ref.signal_emitter())
                .await
            {
                warn!("Failed to send property changed signal for stopped pools on interface Manager.r3: {e}");
            }
        }
        Err(e) => {
            warn!("Failed to get Manager.r3 interface: {e}");
        }
    }

    match connection
        .object_server()
        .interface::<_, ManagerR4>(&path)
        .await
    {
        Ok(iface_ref) => {
            let mut_iface_ref = iface_ref.get_mut().await;
            if let Err(e) = mut_iface_ref
                .stopped_pools_changed(iface_ref.signal_emitter())
                .await
            {
                warn!("Failed to send property changed signal for stopped pools on interface Manager.r4: {e}");
            }
        }
        Err(e) => {
            warn!("Failed to get Manager.r4 interface: {e}");
        }
    }

    match connection
        .object_server()
        .interface::<_, ManagerR5>(&path)
        .await
    {
        Ok(iface_ref) => {
            let mut_iface_ref = iface_ref.get_mut().await;
            if let Err(e) = mut_iface_ref
                .stopped_pools_changed(iface_ref.signal_emitter())
                .await
            {
                warn!("Failed to send property changed signal for stopped pools on interface Manager.r5: {e}");
            }
        }
        Err(e) => {
            warn!("Failed to get Manager.r5 interface: {e}");
        }
    }

    match connection
        .object_server()
        .interface::<_, ManagerR6>(&path)
        .await
    {
        Ok(iface_ref) => {
            let mut_iface_ref = iface_ref.get_mut().await;
            if let Err(e) = mut_iface_ref
                .stopped_pools_changed(iface_ref.signal_emitter())
                .await
            {
                warn!("Failed to send property changed signal for stopped pools on interface Manager.r6: {e}");
            }
        }
        Err(e) => {
            warn!("Failed to get Manager.r6 interface: {e}");
        }
    }

    match connection
        .object_server()
        .interface::<_, ManagerR7>(&path)
        .await
    {
        Ok(iface_ref) => {
            let mut_iface_ref = iface_ref.get_mut().await;
            if let Err(e) = mut_iface_ref
                .stopped_pools_changed(iface_ref.signal_emitter())
                .await
            {
                warn!("Failed to send property changed signal for stopped pools on interface Manager.r7: {e}");
            }
        }
        Err(e) => {
            warn!("Failed to get Manager.r7 interface: {e}");
        }
    }

    match connection
        .object_server()
        .interface::<_, ManagerR8>(&path)
        .await
    {
        Ok(iface_ref) => {
            let mut_iface_ref = iface_ref.get_mut().await;
            if let Err(e) = mut_iface_ref
                .stopped_pools_changed(iface_ref.signal_emitter())
                .await
            {
                warn!("Failed to send property changed signal for stopped pools on interface Manager.r8: {e}");
            }
        }
        Err(e) => {
            warn!("Failed to get Manager.r8 interface: {e}");
        }
    }

    match connection
        .object_server()
        .interface::<_, ManagerR9>(&path)
        .await
    {
        Ok(iface_ref) => {
            let mut_iface_ref = iface_ref.get_mut().await;
            if let Err(e) = mut_iface_ref
                .stopped_pools_changed(iface_ref.signal_emitter())
                .await
            {
                warn!("Failed to send property changed signal for stopped pools on interface Manager.r9: {e}");
            }
        }
        Err(e) => {
            warn!("Failed to get Manager.r9 interface: {e}");
        }
    }
}

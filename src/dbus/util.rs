// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, fmt::Display, sync::Arc};

use tokio::sync::RwLock;
use zbus::{
    zvariant::{ObjectPath, OwnedObjectPath, Value},
    Connection,
};

use devicemapper::DmError;

use crate::{
    dbus::{
        blockdev::BlockdevInterface,
        consts::STRATIS_BASE_PATH,
        filesystem::FilesystemInterface,
        manager::{Manager, ManagerInterface},
        pool::PoolInterface,
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

macro_rules! send_signal {
    ($connection:expr, $interface:ty, $path:expr, $changed:ident, $signal:literal, $iface:literal) => {
        match $connection
            .object_server()
            .interface::<_, $interface>($path)
            .await
        {
            Ok(iface_ref) => {
                if let Err(e) = iface_ref
                    .get()
                    .await
                    .$changed(iface_ref.signal_emitter())
                    .await
                {
                    warn!(
                        "Failed to send {} signal on interface {}: {e}",
                        $signal, $iface
                    );
                }
            }
            Err(e) => {
                warn!(
                    "Failed to get {} interface to send {} signal: {e}",
                    $iface, $signal
                );
            }
        }
    };
}

#[allow(clippy::implicit_hasher)]
pub async fn send_pool_background_signals(
    manager: Lockable<Arc<RwLock<Manager>>>,
    connection: &Arc<Connection>,
    diffs: HashMap<PoolUuid, PoolDiff>,
) {
    let dbus = manager.read().await;
    for (uuid, diff) in diffs {
        let pool_path = match dbus.pool_get_path(&uuid) {
            Some(path) => path,
            None => {
                warn!("No pool associated with UUID {uuid}, skipping background signals");
                continue;
            }
        };
        if diff.thin_pool.allocated_size.changed().is_some() {
            send_signal!(
                connection,
                PoolInterface,
                pool_path,
                allocated_size_changed,
                "allocated size",
                "pool"
            );
        }
        if diff.thin_pool.used.changed().is_some() {
            send_signal!(
                connection,
                PoolInterface,
                pool_path,
                total_physical_used_changed,
                "total physical used",
                "pool"
            );
        }
        if diff.pool.out_of_alloc_space.changed().is_some() {
            send_signal!(
                connection,
                PoolInterface,
                pool_path,
                no_alloc_space_changed,
                "no alloc space",
                "pool"
            );
        }
        if diff.pool.metadata_size.changed().is_some() {
            send_signal!(
                connection,
                PoolInterface,
                pool_path,
                metadata_used_changed,
                "metadata used",
                "pool"
            );
        }
    }
}

pub async fn send_pool_foreground_signals(
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    uuid: PoolUuid,
    diff: PoolDiff,
) {
    let dbus = manager.read().await;
    let pool_path = match dbus.pool_get_path(&uuid) {
        Some(path) => path,
        None => {
            warn!("No pool associated with UUID {uuid}, skipping allocated_size_changed signal");
            return;
        }
    };
    if diff.thin_pool.allocated_size.changed().is_some() {
        send_signal!(
            connection,
            PoolInterface,
            pool_path,
            allocated_size_changed,
            "allocated size",
            "pool"
        );
    }
    if diff.thin_pool.used.changed().is_some() {
        send_signal!(
            connection,
            PoolInterface,
            pool_path,
            total_physical_used_changed,
            "total physical used",
            "pool"
        );
    }
    if diff.pool.total_physical_size.changed().is_some() {
        send_signal!(
            connection,
            PoolInterface,
            pool_path,
            total_physical_size_changed,
            "total physical size",
            "pool"
        );
    }
    if diff.pool.out_of_alloc_space.changed().is_some() {
        send_signal!(
            connection,
            PoolInterface,
            pool_path,
            no_alloc_space_changed,
            "no alloc space",
            "pool"
        );
    }
    if diff.pool.metadata_size.changed().is_some() {
        send_signal!(
            connection,
            PoolInterface,
            pool_path,
            metadata_used_changed,
            "metadata used",
            "pool"
        );
    }
}

#[allow(clippy::implicit_hasher)]
pub async fn send_fs_background_signals(
    manager: Lockable<Arc<RwLock<Manager>>>,
    connection: &Arc<Connection>,
    diffs: HashMap<FilesystemUuid, StratFilesystemDiff>,
) {
    for (fs_uuid, diff) in diffs {
        match manager.read().await.filesystem_get_path(&fs_uuid) {
            Some(path) => {
                if diff.size.is_changed() {
                    send_fs_size_signal(connection, &path.as_ref()).await;
                }
                if diff.used.is_changed() {
                    send_fs_used_signal(connection, &path.as_ref()).await;
                }
            }
            None => {
                warn!("No filesystem path associated with filesystem UUID {fs_uuid}; cannot send background property change signals");
            }
        }
    }
}

pub async fn send_fs_size_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        FilesystemInterface,
        path,
        size_changed,
        "total size",
        "filesystem"
    );
}

pub async fn send_fs_used_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        FilesystemInterface,
        path,
        used_changed,
        "used size",
        "filesystem"
    );
}

pub async fn send_stopped_pools_signals(connection: &Arc<Connection>) {
    let path = match ObjectPath::from_static_str(STRATIS_BASE_PATH) {
        Ok(path) => path,
        Err(e) => {
            warn!("Failed to create object path for stopped_pools_changed signal: {e}");
            return;
        }
    };

    send_signal!(
        connection,
        ManagerInterface,
        &path,
        stopped_pools_changed,
        "stopped pools",
        "Manager"
    );
}

pub async fn send_pool_name_signal(
    connection: &Arc<Connection>,
    path: &ObjectPath<'_>,
    fs_paths: Vec<&OwnedObjectPath>,
) {
    send_signal!(
        connection,
        PoolInterface,
        path,
        name_changed,
        "name",
        "pool"
    );

    for fs_path_owned in fs_paths {
        let fs_path = &fs_path_owned.as_ref();
        send_signal!(
            connection,
            FilesystemInterface,
            fs_path,
            devnode_invalidate,
            "devnode",
            "filesystem"
        );
    }
}

pub async fn send_overprovisioning_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        PoolInterface,
        path,
        overprovisioning_changed,
        "overprovisioning",
        "pool"
    );
}

pub async fn send_fs_limit_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        PoolInterface,
        path,
        fs_limit_changed,
        "filesystem limit",
        "pool"
    );
}

pub async fn send_clevis_info_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        PoolInterface,
        path,
        clevis_infos_changed,
        "clevis infos",
        "pool"
    );
}

pub async fn send_keyring_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        PoolInterface,
        path,
        key_descriptions_changed,
        "key descriptions",
        "pool"
    );
}

pub async fn send_free_token_slots_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        PoolInterface,
        path,
        free_token_slots_changed,
        "free token slots",
        "pool"
    );
}

pub async fn send_action_availability_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        PoolInterface,
        path,
        available_actions_changed,
        "available actions",
        "pool"
    );
}

pub async fn send_has_cache_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        PoolInterface,
        path,
        has_cache_changed,
        "has cache",
        "pool"
    );
}

pub async fn send_origin_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        FilesystemInterface,
        path,
        origin_changed,
        "origin",
        "filesystem"
    );
}

pub async fn send_size_limit_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        FilesystemInterface,
        path,
        size_limit_changed,
        "size limit",
        "filesystem"
    );
}

pub async fn send_merge_scheduled_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        FilesystemInterface,
        path,
        merge_scheduled_changed,
        "merge scheduled",
        "filesystem"
    );
}

pub async fn send_fs_name_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        FilesystemInterface,
        path,
        name_changed,
        "filesystem name",
        "filesystem"
    );
    send_signal!(
        connection,
        FilesystemInterface,
        path,
        devnode_invalidate,
        "devnode",
        "filesystem"
    );
}

pub async fn send_new_physical_size_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        BlockdevInterface,
        path,
        new_physical_size_changed,
        "blockdev physical size",
        "blockdev"
    );
}

pub async fn send_user_info_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        BlockdevInterface,
        path,
        user_info_changed,
        "user info",
        "blockdev"
    );
}

pub async fn send_encrypted_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        PoolInterface,
        path,
        encrypted_changed,
        "encryption status",
        "pool"
    );
}

pub async fn send_last_reencrypted_signal(connection: &Arc<Connection>, path: &ObjectPath<'_>) {
    send_signal!(
        connection,
        PoolInterface,
        path,
        last_reencrypted_timestamp_changed,
        "last reencryption timestamp",
        "pool"
    );
}

pub async fn send_blockdev_physical_size_signal(
    connection: &Arc<Connection>,
    path: &ObjectPath<'_>,
) {
    send_signal!(
        connection,
        BlockdevInterface,
        path,
        total_physical_size_changed,
        "total physical size",
        "blockdev"
    );
}

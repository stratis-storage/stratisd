// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use chrono::SecondsFormat;
use devicemapper::Bytes;
use tokio::sync::{OwnedRwLockReadGuard, RwLock};
use zbus::{fdo::Error, zvariant::OwnedObjectPath, Connection};

use crate::{
    dbus::{
        manager::Manager,
        util::{
            option_to_tuple, send_merge_scheduled_signal, send_size_limit_signal, tuple_to_option,
        },
    },
    engine::{
        Filesystem, FilesystemUuid, Lockable, Name, Pool, PoolUuid, PropChangeAction, SharedGuard,
        SomeLockWriteGuard,
    },
};

pub fn created_prop(_: Name, _: Name, _: FilesystemUuid, fs: &dyn Filesystem) -> String {
    fs.created().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn devnode_prop(
    pool_name: Name,
    fs_name: Name,
    _: FilesystemUuid,
    fs: &dyn Filesystem,
) -> String {
    fs.path_to_mount_filesystem(&pool_name, &fs_name)
        .display()
        .to_string()
}

pub fn name_prop(_: Name, name: Name, _: FilesystemUuid, _: &dyn Filesystem) -> Name {
    name
}

pub fn pool_prop(
    guard: SharedGuard<OwnedRwLockReadGuard<Manager>>,
    pool_uuid: PoolUuid,
) -> Result<OwnedObjectPath, Error> {
    guard
        .pool_get_path(&pool_uuid)
        .ok_or_else(|| {
            Error::Failed(format!(
                "No object path associated with pool UUID {pool_uuid}"
            ))
        })
        .cloned()
}

pub fn size_prop(_: Name, _: Name, _: FilesystemUuid, fs: &dyn Filesystem) -> String {
    (*fs.size()).to_string()
}

pub fn used_prop(_: Name, _: Name, _: FilesystemUuid, fs: &dyn Filesystem) -> (bool, String) {
    option_to_tuple(fs.used().ok().map(|u| (*u).to_string()), String::new())
}

pub fn size_limit_prop(_: Name, _: Name, _: FilesystemUuid, fs: &dyn Filesystem) -> (bool, String) {
    option_to_tuple(
        fs.size_limit().map(|u| (*u.bytes()).to_string()),
        String::new(),
    )
}

pub async fn set_size_limit_prop(
    mut guard: SomeLockWriteGuard<PoolUuid, dyn Pool>,
    fs_uuid: FilesystemUuid,
    size_limit_tuple: (bool, String),
) -> Result<bool, Error> {
    let size_limit_str = tuple_to_option(size_limit_tuple);
    let size_limit = match size_limit_str {
        Some(lim) => Some(Bytes(lim.parse::<u128>().map_err(|e| {
            Error::Failed(format!("Failed to parse {lim} as unsigned integer: {e}"))
        })?)),
        None => None,
    };
    let (_, _, pool) = guard.as_mut_tuple();
    match pool.set_fs_size_limit(fs_uuid, size_limit) {
        Ok(PropChangeAction::NewValue(_v)) => Ok(true),
        Ok(PropChangeAction::Identity) => Ok(false),
        Err(e) => Err(Error::Failed(e.to_string())),
    }
}

pub async fn send_size_limit_signal_on_change(
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    fs_uuid: FilesystemUuid,
) {
    match manager.read().await.filesystem_get_path(&fs_uuid) {
        Some(p) => send_size_limit_signal(connection, &p.as_ref()).await,
        None => {
            warn!("No path associated with filesystem UUID {fs_uuid}; cannot send property changed signal");
        }
    }
}

pub fn origin_prop(_: Name, _: Name, _: FilesystemUuid, fs: &dyn Filesystem) -> (bool, String) {
    let (has_origin, uuid) = option_to_tuple(fs.origin(), FilesystemUuid::nil());
    (has_origin, uuid.simple().to_string())
}

pub fn merge_scheduled_prop(_: Name, _: Name, _: FilesystemUuid, fs: &dyn Filesystem) -> bool {
    fs.merge_scheduled()
}

pub async fn set_merge_scheduled_prop(
    mut guard: SomeLockWriteGuard<PoolUuid, dyn Pool>,
    fs_uuid: FilesystemUuid,
    scheduled: bool,
) -> Result<bool, Error> {
    let (_, _, pool) = guard.as_mut_tuple();
    match pool.set_fs_merge_scheduled(fs_uuid, scheduled) {
        Ok(PropChangeAction::NewValue(_v)) => Ok(true),
        Ok(PropChangeAction::Identity) => Ok(false),
        Err(e) => Err(Error::Failed(e.to_string())),
    }
}

pub async fn send_merge_scheduled_signal_on_change(
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    fs_uuid: FilesystemUuid,
) {
    match manager.read().await.filesystem_get_path(&fs_uuid) {
        Some(p) => send_merge_scheduled_signal(connection, &p.as_ref()).await,
        None => {
            warn!("No path associated with filesystem UUID {fs_uuid}; cannot send property changed signal");
        }
    }
}

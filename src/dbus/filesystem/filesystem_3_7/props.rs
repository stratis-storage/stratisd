// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use tokio::sync::RwLock;
use zbus::{Connection, Error};

use crate::{
    dbus::{
        manager::Manager,
        util::{option_to_tuple, send_merge_scheduled_signal},
    },
    engine::{
        Filesystem, FilesystemUuid, Lockable, Name, Pool, PoolUuid, PropChangeAction,
        SomeLockWriteGuard,
    },
};

pub fn origin_prop(
    _: Name,
    _: Name,
    _: FilesystemUuid,
    fs: &dyn Filesystem,
) -> (bool, FilesystemUuid) {
    option_to_tuple(fs.origin(), FilesystemUuid::nil())
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
        Err(e) => Err(Error::Failure(e.to_string())),
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

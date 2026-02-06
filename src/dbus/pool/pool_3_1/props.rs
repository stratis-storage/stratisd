// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use tokio::sync::RwLock;
use zbus::{Connection, Error};

use crate::{
    dbus::{
        manager::Manager,
        util::{send_fs_limit_signal, send_overprovisioning_signal},
    },
    engine::{Lockable, Pool, PoolUuid, SomeLockReadGuard, SomeLockWriteGuard},
};

pub fn fs_limit_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> u64 {
    guard.fs_limit()
}

pub async fn set_fs_limit_prop(
    mut guard: SomeLockWriteGuard<PoolUuid, dyn Pool>,
    uuid: PoolUuid,
    fs_limit: u64,
) -> Result<bool, Error> {
    let (name, _, p) = guard.as_mut_tuple();
    p.set_fs_limit(&name, uuid, fs_limit)
        .map(|_| true)
        .map_err(|e| Error::Failure(e.to_string()))
}

pub async fn send_fs_limit_signal_on_change(
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    uuid: PoolUuid,
) {
    match manager.read().await.pool_get_path(&uuid) {
        Some(p) => send_fs_limit_signal(connection, p).await,
        None => {
            warn!("No path associated with UUID {uuid}; cannot send property changed signal");
        }
    }
}

pub fn enable_overprovisioning_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> bool {
    guard.overprov_enabled()
}

pub async fn set_enable_overprovisioning_prop(
    mut guard: SomeLockWriteGuard<PoolUuid, dyn Pool>,
    _uuid: PoolUuid,
    enable_overprov: bool,
) -> Result<bool, Error> {
    let (name, _, p) = guard.as_mut_tuple();
    p.set_overprov_mode(&name, enable_overprov)
        .map(|_| true)
        .map_err(|e| Error::Failure(e.to_string()))
}

pub async fn send_enable_overprovisioning_signal_on_change(
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    uuid: PoolUuid,
) {
    match manager.read().await.pool_get_path(&uuid) {
        Some(p) => send_overprovisioning_signal(connection, p).await,
        None => {
            warn!("No path associated with UUID {uuid}; cannot send property changed signal");
        }
    }
}

pub fn no_alloc_space_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> bool {
    guard.out_of_alloc_space()
}

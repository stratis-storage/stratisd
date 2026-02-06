// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use tokio::sync::RwLock;
use zbus::{Connection, Error};

use crate::{
    dbus::{
        manager::Manager,
        util::{option_to_tuple, send_user_info_signal, tuple_to_option},
    },
    engine::{BlockDev, BlockDevTier, DevUuid, Lockable, Pool, PoolUuid, SomeLockWriteGuard},
};

pub fn new_physical_size_prop(_: BlockDevTier, _: DevUuid, dev: &dyn BlockDev) -> (bool, String) {
    option_to_tuple(
        dev.new_size().map(|s| (*s.bytes()).to_string()),
        String::new(),
    )
}

pub async fn set_user_info_prop(
    mut guard: SomeLockWriteGuard<PoolUuid, dyn Pool>,
    dev_uuid: DevUuid,
    user_info_tuple: (bool, String),
) -> Result<bool, Error> {
    let user_info = tuple_to_option(user_info_tuple);
    let user_info = user_info.as_deref();
    let (pool_name, _, pool) = guard.as_mut_tuple();
    if pool
        .get_blockdev(dev_uuid)
        .ok_or_else(|| Error::Failure(format!("Blockdev with UUID {dev_uuid} not found")))?
        .1
        .user_info()
        == user_info
    {
        Ok(false)
    } else {
        pool.set_blockdev_user_info(&pool_name, dev_uuid, user_info)
            .map_err(|e| Error::Failure(e.to_string()))?;
        Ok(true)
    }
}

pub async fn send_user_info_signal_on_change(
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    dev_uuid: DevUuid,
) {
    match manager.read().await.blockdev_get_path(&dev_uuid) {
        Some(p) => send_user_info_signal(connection, &p.as_ref()).await,
        None => {
            warn!("No path associated with blockdev UUID {dev_uuid}; cannot send property changed signal");
        }
    }
}

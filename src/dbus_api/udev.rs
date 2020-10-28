// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::mpsc::Receiver;

use crate::{
    dbus_api::{
        blockdev::create_dbus_blockdev, filesystem::create_dbus_filesystem, pool::create_dbus_pool,
        types::DbusContext,
    },
    engine::{Name, Pool, PoolUuid, UdevEngineEvent},
    stratis::StratisResult,
};

pub struct DbusUdevHandler {
    pub(super) receiver: Receiver<UdevEngineEvent>,
    pub(super) path: dbus::Path<'static>,
    pub(super) dbus_context: DbusContext,
}

impl DbusUdevHandler {
    /// Process udev events using D-Bus.
    pub fn handle_udev_event(&mut self) -> StratisResult<()> {
        let udev_event = self.receiver.recv()?;
        let mut mutex_lock = self.dbus_context.engine.lock()?;
        let optional_pool_info = (*mutex_lock).handle_event(&udev_event);

        if let Some((pool_name, pool_uuid, pool)) = optional_pool_info {
            self.register_pool(&pool_name, pool_uuid, pool)
        }
        Ok(())
    }

    /// Register a pool in the engine with D-Bus.
    pub fn register_pool(&self, pool_name: &Name, pool_uuid: PoolUuid, pool: &mut dyn Pool) {
        let pool_path = create_dbus_pool(
            &self.dbus_context,
            self.path.clone(),
            pool_name,
            pool_uuid,
            pool,
        );
        for (fs_name, fs_uuid, fs) in pool.filesystems_mut() {
            create_dbus_filesystem(
                &self.dbus_context,
                pool_path.clone(),
                pool_name,
                &fs_name,
                fs_uuid,
                fs,
            );
        }
        for (uuid, tier, bd) in pool.blockdevs_mut() {
            create_dbus_blockdev(&self.dbus_context, pool_path.clone(), uuid, tier, bd);
        }
    }
}

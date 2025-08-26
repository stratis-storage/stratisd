// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::task::Poll;

use futures::{pin_mut, poll};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    dbus_api::{
        blockdev::create_dbus_blockdev,
        filesystem::create_dbus_filesystem,
        pool::create_dbus_pool,
        types::{DbusAction, DbusContext},
    },
    engine::{Name, Pool, PoolUuid, UdevEngineEvent},
    stratis::{StratisError, StratisResult},
};

pub struct DbusUdevHandler {
    pub(super) receiver: UnboundedReceiver<UdevEngineEvent>,
    pub(super) path: dbus::Path<'static>,
    pub(super) dbus_context: DbusContext,
}

impl DbusUdevHandler {
    pub fn new(
        receiver: UnboundedReceiver<UdevEngineEvent>,
        path: dbus::Path<'static>,
        dbus_context: DbusContext,
    ) -> Self {
        DbusUdevHandler {
            receiver,
            path,
            dbus_context,
        }
    }

    /// Process udev events that were detected on the udev socket.
    pub async fn handle_udev_event(&mut self) -> StratisResult<()> {
        let mut events = Vec::new();
        events.push(self.receiver.recv().await.ok_or_else(|| {
            StratisError::Msg("Channel from udev handler to D-Bus handler was shut".to_string())
        })?);

        // Avoiding implementing DumpState here because all of the information
        // is cached, the entire property will be considered changed if any
        // part changes, and it generally makes more sense to treat the
        // HashMap comparison as a diff in itself.
        let (original_locked_state, original_stopped_state) = (
            self.dbus_context.engine.locked_pools().await,
            self.dbus_context.engine.stopped_pools().await,
        );

        loop {
            let recv = self.receiver.recv();
            pin_mut!(recv);
            match poll!(recv) {
                Poll::Ready(Some(event)) => events.push(event),
                Poll::Ready(None) => {
                    return Err(StratisError::Msg(
                        "Channel from udev handler to D-Bus handler was shut".to_string(),
                    ));
                }
                Poll::Pending => break,
            }
        }

        let (pool_infos, dev_infos) = self.dbus_context.engine.handle_events(events).await;
        for guard in pool_infos {
            let (pool_name, pool_uuid, pool) = guard.as_tuple();
            self.register_pool(&pool_name, pool_uuid, pool);
        }

        let new_locked_state = self.dbus_context.engine.locked_pools().await;
        if original_locked_state != new_locked_state {
            self.dbus_context.push_locked_pools(new_locked_state);
        }
        let new_stopped_state = self.dbus_context.engine.stopped_pools().await;
        if original_stopped_state != new_stopped_state {
            self.dbus_context.push_stopped_pools(new_stopped_state);
        }

        for action in DbusAction::from_bd_diffs(dev_infos) {
            if let Err(e) = self.dbus_context.sender.send(action) {
                warn!("Failed to update D-Bus layer with block device event changes: {e}");
            }
        }

        Ok(())
    }

    /// Register a pool in the engine with D-Bus.
    pub fn register_pool(&self, pool_name: &Name, pool_uuid: PoolUuid, pool: &dyn Pool) {
        let pool_path = create_dbus_pool(
            &self.dbus_context,
            self.path.clone(),
            pool_name,
            pool_uuid,
            pool,
        );
        for (fs_name, fs_uuid, fs) in pool.filesystems() {
            create_dbus_filesystem(
                &self.dbus_context,
                pool_path.clone(),
                pool_name,
                &fs_name,
                fs_uuid,
                fs,
            );
        }
        for (uuid, tier, bd) in pool.blockdevs() {
            create_dbus_blockdev(&self.dbus_context, pool_path.clone(), uuid, tier, bd);
        }
    }
}

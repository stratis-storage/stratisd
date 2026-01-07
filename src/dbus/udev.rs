// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{atomic::AtomicU64, Arc};

use tokio::sync::{mpsc::UnboundedReceiver, RwLock};
use zbus::Connection;

use crate::{
    dbus::{manager::Manager, pool::register_pool, util::send_new_physical_size_signal},
    engine::{Engine, Lockable, PoolUuid, UdevEngineEvent},
    stratis::{StratisError, StratisResult},
};

pub struct UdevHandler {
    connection: Arc<Connection>,
    engine: Arc<dyn Engine>,
    manager: Lockable<Arc<RwLock<Manager>>>,
    receiver: UnboundedReceiver<UdevEngineEvent>,
    counter: Arc<AtomicU64>,
}

impl UdevHandler {
    pub fn new(
        connection: Arc<Connection>,
        engine: Arc<dyn Engine>,
        manager: Lockable<Arc<RwLock<Manager>>>,
        receiver: UnboundedReceiver<UdevEngineEvent>,
        counter: Arc<AtomicU64>,
    ) -> Self {
        UdevHandler {
            connection,
            engine,
            manager,
            receiver,
            counter,
        }
    }

    pub async fn process_udev_events(&mut self) -> StratisResult<()> {
        let mut events = Vec::new();
        events.push(self.receiver.recv().await.ok_or_else(|| {
            StratisError::Msg("Channel from udev handler to D-Bus handler was shut".to_string())
        })?);

        while let Ok(event) = self.receiver.try_recv() {
            events.push(event);
        }

        let (pool_infos, dev_infos) = self.engine.handle_events(events).await;
        for guard in pool_infos {
            let (_, pool_uuid, _) = guard.as_tuple();
            if let Err(e) = self.register_pool(pool_uuid).await {
                warn!("Failed to register pool: {e}");
            }
        }

        for (dev_uuid, diff) in dev_infos {
            if diff.size.is_changed() {
                match self.manager.read().await.blockdev_get_path(&dev_uuid) {
                    Some(p) => send_new_physical_size_signal(&self.connection, &p.as_ref()).await,
                    None => {
                        warn!("No path was found for blockdev UUID {dev_uuid}; cannot send blockdev physical size change signal");
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn register_pool(&self, pool_uuid: PoolUuid) -> StratisResult<()> {
        register_pool(
            &self.engine,
            &self.connection,
            &self.manager,
            &self.counter,
            pool_uuid,
        )
        .await?;
        Ok(())
    }
}

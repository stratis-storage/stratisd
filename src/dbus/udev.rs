// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{atomic::AtomicU64, Arc};

use tokio::sync::mpsc::UnboundedReceiver;
use zbus::Connection;

use crate::{
    dbus::pool::register_pool,
    engine::{Engine, PoolUuid, UdevEngineEvent},
    stratis::{StratisError, StratisResult},
};

pub struct UdevHandler {
    connection: Arc<Connection>,
    engine: Arc<dyn Engine>,
    receiver: UnboundedReceiver<UdevEngineEvent>,
    counter: Arc<AtomicU64>,
}

impl UdevHandler {
    pub fn new(
        connection: Arc<Connection>,
        engine: Arc<dyn Engine>,
        receiver: UnboundedReceiver<UdevEngineEvent>,
        counter: Arc<AtomicU64>,
    ) -> Self {
        UdevHandler {
            connection,
            engine,
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

        let (pool_infos, _) = self.engine.handle_events(events).await;
        for guard in pool_infos {
            let (_, pool_uuid, _) = guard.as_tuple();
            if let Err(e) = self.register_pool(pool_uuid).await {
                warn!("Failed to register pool: {e}");
            }
        }

        Ok(())
    }

    pub async fn register_pool(&self, pool_uuid: PoolUuid) -> StratisResult<()> {
        register_pool(
            &self.connection,
            &self.counter,
            Arc::clone(&self.engine),
            pool_uuid,
        )
        .await?;
        Ok(())
    }
}

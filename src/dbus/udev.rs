// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{future::pending, sync::Arc};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use zbus::Connection;

use crate::{
    dbus::types::DbusAction,
    engine::{Name, Pool, PoolUuid, UdevEngineEvent},
    stratis::StratisResult,
};

pub struct UdevHandler {
    _connection: Arc<Connection>,
    _receiver: UnboundedReceiver<UdevEngineEvent>,
    _sender: UnboundedSender<DbusAction>,
}

impl UdevHandler {
    pub fn new(
        connection: Arc<Connection>,
        receiver: UnboundedReceiver<UdevEngineEvent>,
        sender: UnboundedSender<DbusAction>,
    ) -> Self {
        UdevHandler {
            _connection: connection,
            _receiver: receiver,
            _sender: sender,
        }
    }

    pub async fn register_pool(&self, _name: &Name, _uuid: PoolUuid, _pool: &dyn Pool) {}

    pub async fn handle_udev_event(&mut self) -> StratisResult<()> {
        pending().await
    }
}

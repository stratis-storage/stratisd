// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{future::pending, sync::Arc};

use tokio::sync::mpsc::UnboundedReceiver;
use zbus::Connection;

use crate::{dbus::types::DbusAction, stratis::StratisResult};

pub struct SignalHandler {
    _connection: Arc<Connection>,
    _receiver: UnboundedReceiver<DbusAction>,
}

impl SignalHandler {
    pub fn new(connection: Arc<Connection>, receiver: UnboundedReceiver<DbusAction>) -> Self {
        SignalHandler {
            _connection: connection,
            _receiver: receiver,
        }
    }

    pub async fn process_dbus_actions(&mut self) -> StratisResult<()> {
        pending().await
    }
}

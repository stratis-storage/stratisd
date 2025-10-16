// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;
use zbus::interface;

use crate::{dbus::types::DbusAction, engine::Engine, stratis::VERSION};

pub struct ManagerR9 {
    _engine: Arc<dyn Engine>,
    _receiver: UnboundedSender<DbusAction>,
}

impl ManagerR9 {
    pub fn new(engine: Arc<dyn Engine>, receiver: UnboundedSender<DbusAction>) -> Self {
        ManagerR9 {
            _engine: engine,
            _receiver: receiver,
        }
    }
}

#[interface(name = "org.storage.stratis3.Manager.r9")]
impl ManagerR9 {
    #[zbus(property(emits_changed_signal = "const"))]
    #[allow(non_snake_case)]
    #[allow(clippy::unused_self)]
    fn Version(&self) -> &str {
        VERSION
    }
}

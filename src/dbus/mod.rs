// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use zbus::connection::Builder;

pub use crate::dbus::types::DbusAction;
use crate::{
    dbus::{manager::ManagerR9, signal::SignalHandler, udev::UdevHandler},
    engine::{Engine, UdevEngineEvent},
    stratis::StratisResult,
};

mod consts;
mod manager;
mod signal;
mod types;
mod udev;

pub async fn create_dbus_handlers(
    engine: Arc<dyn Engine>,
    udev_receiver: UnboundedReceiver<UdevEngineEvent>,
    (sender, receiver): (UnboundedSender<DbusAction>, UnboundedReceiver<DbusAction>),
) -> StratisResult<(UdevHandler, SignalHandler)> {
    let manager = ManagerR9::new(Arc::clone(&engine), sender.clone());

    let connection = Arc::new(
        Builder::system()?
            .serve_at(consts::STRATIS_BASE_PATH, manager)?
            .name(consts::STRATIS_BASE_SERVICE)?
            .build()
            .await?,
    );
    Ok((
        UdevHandler::new(Arc::clone(&connection), udev_receiver, sender),
        SignalHandler::new(connection, receiver),
    ))
}

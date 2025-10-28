// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{atomic::AtomicU64, Arc};

use tokio::sync::mpsc::UnboundedReceiver;
use zbus::{connection::Builder, Connection};

pub use crate::dbus::{
    udev::UdevHandler,
    util::{send_fs_background_signals, send_pool_background_signals},
};
use crate::{
    dbus::manager::ManagerR9,
    engine::{Engine, UdevEngineEvent},
    stratis::StratisResult,
};

mod consts;
mod manager;
mod pool;
mod types;
mod udev;
mod util;

pub async fn create_dbus_handler(
    engine: Arc<dyn Engine>,
    receiver: UnboundedReceiver<UdevEngineEvent>,
) -> StratisResult<(Arc<Connection>, UdevHandler)> {
    let counter = Arc::new(AtomicU64::new(0));
    let connection = Arc::new(
        Builder::system()?
            .name(consts::STRATIS_BASE_SERVICE)?
            .build()
            .await?,
    );
    ManagerR9::register(&connection, Arc::clone(&engine), Arc::clone(&counter)).await?;
    let udev = UdevHandler::new(Arc::clone(&connection), engine, receiver, counter);
    Ok((connection, udev))
}

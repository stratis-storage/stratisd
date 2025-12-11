// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{atomic::AtomicU64, Arc};

use tokio::sync::{mpsc::UnboundedReceiver, RwLock};
use zbus::{connection::Builder, Connection};

pub use crate::dbus::{
    manager::Manager,
    udev::UdevHandler,
    util::{send_fs_background_signals, send_pool_background_signals},
};
use crate::{
    dbus::manager::register_manager,
    engine::{Engine, Lockable, UdevEngineEvent},
    stratis::StratisResult,
};

#[macro_use]
mod macros;

mod blockdev;
mod consts;
mod filesystem;
mod manager;
mod pool;
mod types;
mod udev;
mod util;

pub async fn create_dbus_handler(
    engine: Arc<dyn Engine>,
    receiver: UnboundedReceiver<UdevEngineEvent>,
) -> StratisResult<(Arc<Connection>, UdevHandler, Lockable<Arc<RwLock<Manager>>>)> {
    let counter = Arc::new(AtomicU64::new(0));
    let manager = Lockable::new_shared(Manager::default());
    let connection = Arc::new(
        Builder::system()?
            .name(consts::STRATIS_BASE_SERVICE)?
            .build()
            .await?,
    );
    register_manager(&connection, &engine, &manager, &counter).await;
    let udev = UdevHandler::new(
        Arc::clone(&connection),
        engine,
        manager.clone(),
        receiver,
        counter,
    );
    Ok((connection, udev, manager))
}

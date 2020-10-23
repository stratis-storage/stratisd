// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Conditionally compiled support for a D-Bus interface.

// Allow new_without_default lint, because otherwise it would be necessary
// to implement Default twice, one implementation for the supported version
// and one for the unsupported version. Also, Default is not really a
// helpful concept here.

use std::sync::{atomic::AtomicBool, Arc};

use tokio::{
    select,
    sync::{mpsc::Receiver, Mutex},
    task,
};

use crate::{
    dbus_api::{create_dbus_handlers, EventHandler},
    engine::{get_engine_listener_list_mut, Engine, UdevEngineEvent},
    stratis::{StratisError, StratisResult},
};

pub async fn setup(
    engine: Arc<Mutex<dyn Engine>>,
    receiver: Receiver<UdevEngineEvent>,
    should_exit: Arc<AtomicBool>,
) -> StratisResult<()> {
    let (conn, mut udev, mut tree) =
        create_dbus_handlers(Arc::clone(&engine), receiver, should_exit)
            .map_err(|err| -> StratisError { err.into() })
            .map(|(conn, udev, tree)| {
                let event_handler = Box::new(EventHandler::new(conn.new_connection_ref()));
                get_engine_listener_list_mut().register_listener(event_handler);
                let mut mutex_lock = mutex_lock!(engine);
                for (pool_name, pool_uuid, pool) in mutex_lock.pools_mut() {
                    udev.register_pool(&pool_name, pool_uuid, pool)
                }
                info!("D-Bus API is available");
                (conn, udev, tree)
            })?;

    let mut tree_handle = task::spawn(async move {
        if let Err(e) = tree.process_dbus_actions().await {
            error!(
                "Failed to process D-Bus object path addition or removal: {}; \
                exiting D-Bus thread",
                e,
            );
            return;
        }
    });
    let mut conn_handle = task::spawn_blocking(move || loop {
        match conn.process_dbus_request() {
            Ok(true) => {
                info!("D-Bus request thread was notified to exit");
                return;
            }
            Ok(_) => (),
            Err(e) => {
                error!(
                    "Failed to process D-Bus method call: {}; exiting D-Bus thread",
                    e,
                );
                return;
            }
        }
    });
    let mut udev_handle = task::spawn(async move {
        loop {
            if let Err(e) = udev.handle_udev_event().await {
                error!(
                    "Failed to process udev event in the D-Bus layer: {}; exiting D-Bus thread",
                    e,
                );
                return;
            }
        }
    });

    select! {
        res = &mut tree_handle => {
            error!("The tree handling thread exited...");
            res.map_err(|e| StratisError::Error(e.to_string()))
        }
        res = &mut conn_handle => {
            error!("The D-Bus request thread exited...");
            res.map_err(|e| StratisError::Error(e.to_string()))
        }
        res = &mut udev_handle => {
            error!("The udev processing thread exited...");
            res.map_err(|e| StratisError::Error(e.to_string()))
        }
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Conditionally compiled support for a D-Bus interface.

// Allow new_without_default lint, because otherwise it would be necessary
// to implement Default twice, one implementation for the supported version
// and one for the unsupported version. Also, Default is not really a
// helpful concept here.

#![allow(dead_code)]

use std::sync::{mpsc::Receiver, Arc, Mutex};

use tokio::{select, task};

use crate::{
    dbus_api::{create_dbus_handlers, EventHandler},
    engine::{get_engine_listener_list_mut, Engine, UdevEngineEvent},
    stratis::{StratisError, StratisResult},
};

pub async fn setup(
    engine: Arc<Mutex<dyn Engine>>,
    receiver: Receiver<UdevEngineEvent>,
) -> StratisResult<()> {
    let (conn, mut udev, mut tree) = create_dbus_handlers(Arc::clone(&engine), receiver)
        .map_err(|err| err.into())
        .and_then(|(conn, udev, tree)| -> StratisResult<_> {
            let event_handler = Box::new(EventHandler::new(conn.new_connection_ref()));
            get_engine_listener_list_mut().register_listener(event_handler);
            let mut mutex_lock = mutex_lock!(engine);
            let engine_ref = &mut *mutex_lock;
            for (pool_name, pool_uuid, pool) in engine_ref.pools_mut() {
                udev.register_pool(&pool_name, pool_uuid, pool)
            }
            info!("D-Bus API is available");
            Ok((conn, udev, tree))
        })?;

    let mut tree_handle = task::spawn_blocking(move || loop {
        if let Err(e) = tree.process_dbus_action() {
            error!(
                "Failed to process D-Bus object path addition or removal: {}; \
                    exiting D-Bus thread",
                e,
            );
            return;
        }
    });
    let mut conn_handle = task::spawn_blocking(move || loop {
        if let Err(e) = conn.process_dbus_request() {
            error!(
                "Failed to process D-Bus method call: {}; exiting D-Bus thread",
                e,
            );
            return;
        }
    });
    let mut udev_handle = task::spawn_blocking(move || loop {
        if let Err(e) = udev.handle_udev_event() {
            error!(
                "Failed to process udev event in the D-Bus layer: {}; exiting D-Bus \
                    thread",
                e,
            );
            return;
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

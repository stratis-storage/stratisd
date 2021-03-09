// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Conditionally compiled support for a D-Bus interface.

// Allow new_without_default lint, because otherwise it would be necessary
// to implement Default twice, one implementation for the supported version
// and one for the unsupported version. Also, Default is not really a
// helpful concept here.

use std::sync::Arc;

use tokio::{
    select,
    sync::{mpsc::UnboundedReceiver, Mutex},
    task,
};

use crate::{
    dbus_api::create_dbus_handlers,
    engine::{Engine, UdevEngineEvent},
    stratis::{StratisError, StratisResult},
};

/// Set up the cooperating D-Bus threads.
pub async fn setup(
    engine: Arc<Mutex<dyn Engine>>,
    receiver: UnboundedReceiver<UdevEngineEvent>,
) -> StratisResult<()> {
    let (conn, mut udev, mut tree) = create_dbus_handlers(Arc::clone(&engine), receiver)
        .await
        .map(|(conn, udev, tree)| {
            let mutex_lock = mutex_lock!(engine);
            for (pool_name, pool_uuid, pool) in mutex_lock.pools() {
                udev.register_pool(&pool_name, pool_uuid, pool)
            }
            info!("D-Bus API is available");
            (conn, udev, tree)
        })
        .map_err(|err| -> StratisError { err.into() })?;

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
    let mut conn_handle = task::spawn(async move { conn.process_dbus_requests().await });
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
            res.map_err(|e| StratisError::Error(e.to_string())).and_then(|res| res)
        }
        res = &mut udev_handle => {
            error!("The udev processing thread exited...");
            res.map_err(|e| StratisError::Error(e.to_string()))
        }
    }
}

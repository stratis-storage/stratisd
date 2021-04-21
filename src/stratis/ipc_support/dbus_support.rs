// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Conditionally compiled support for a D-Bus interface.

// Allow new_without_default lint, because otherwise it would be necessary
// to implement Default twice, one implementation for the supported version
// and one for the unsupported version. Also, Default is not really a
// helpful concept here.

use tokio::{select, sync::mpsc::UnboundedReceiver, task};

use crate::{
    dbus_api::create_dbus_handlers,
    engine::{Engine, Locked, UdevEngineEvent},
    stratis::{StratisError, StratisResult},
};

/// Set up the cooperating D-Bus threads.
pub async fn setup(
    engine: Locked<dyn Engine>,
    receiver: UnboundedReceiver<UdevEngineEvent>,
) -> StratisResult<()> {
    let (conn, mut udev, mut tree) = match create_dbus_handlers(engine.clone(), receiver).await {
        Ok((conn, udev, tree)) => {
            let lock = engine.read().await;
            for (pool_name, pool_uuid, pool) in lock.pools() {
                let pool_ref = &*pool.read().await;
                udev.register_pool(&pool_name, pool_uuid, pool_ref)
            }
            info!("D-Bus API is available");
            (conn, udev, tree)
        }
        Err(err) => return Err(StratisError::from(err)),
    };

    let mut tree_handle = task::spawn(async move {
        loop {
            if let Err(e) = tree.process_dbus_actions().await {
                error!(
                    "Failed to process D-Bus object path addition or removal: {}; \
                    exiting D-Bus thread",
                    e,
                );
                return;
            }
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

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
    sync::{
        broadcast::Sender,
        mpsc::{UnboundedReceiver, UnboundedSender},
    },
    task,
};

use crate::{
    dbus_api::{create_dbus_handlers, DbusAction},
    engine::{Engine, UdevEngineEvent},
    stratis::{StratisError, StratisResult},
};

/// Set up the cooperating D-Bus threads.
pub async fn setup<E>(
    engine: Arc<E>,
    receiver: UnboundedReceiver<UdevEngineEvent>,
    trigger: Sender<()>,
    tree_channel: (
        UnboundedSender<DbusAction<E>>,
        UnboundedReceiver<DbusAction<E>>,
    ),
) -> StratisResult<()>
where
    E: 'static + Engine,
{
    let engine_clone = Arc::clone(&engine);
    let (mut conn, udev, mut tree) =
        spawn_blocking!({ create_dbus_handlers(engine_clone, receiver, trigger, tree_channel) })??;

    let pools = engine.pools().await;
    let mut udev = spawn_blocking!({
        for (pool_name, pool_uuid, pool) in pools.iter() {
            udev.register_pool(pool_name, *pool_uuid, pool);
        }
        udev
    })?;
    info!("D-Bus API is available");

    let mut tree_handle = task::spawn_blocking(move || {
        if let Err(e) = tree.process_dbus_actions() {
            error!(
                "Failed to process D-Bus object path addition or removal: {}; \
                exiting D-Bus thread",
                e,
            );
        }
    });
    let mut conn_handle = task::spawn_blocking(move || conn.process_dbus_requests());
    let mut udev_handle = task::spawn(async move {
        loop {
            debug!("Starting D-Bus udev event handling");
            if let Err(e) = udev.handle_udev_event().await {
                error!(
                    "Failed to process udev event in the D-Bus layer: {}; exiting D-Bus thread",
                    e,
                );
                return;
            }
            debug!("Finished D-Bus udev event handling");
        }
    });

    select! {
        res = &mut tree_handle => {
            error!("The tree handling thread exited...");
            res.map_err(StratisError::from)
        }
        res = &mut conn_handle => {
            error!("The D-Bus request thread exited...");
            res.map_err(StratisError::from).and_then(|res| res)
        }
        res = &mut udev_handle => {
            error!("The udev processing thread exited...");
            res.map_err(StratisError::from)
        }
    }
}

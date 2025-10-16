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
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
    task,
};

use crate::{
    dbus::{create_dbus_handlers, DbusAction},
    engine::{Engine, UdevEngineEvent},
    stratis::{StratisError, StratisResult},
};

/// Set up the cooperating D-Bus threads.
pub async fn setup(
    engine: Arc<dyn Engine>,
    receiver: UnboundedReceiver<UdevEngineEvent>,
    tree_channel: (UnboundedSender<DbusAction>, UnboundedReceiver<DbusAction>),
) -> StratisResult<()> {
    let engine_clone = Arc::clone(&engine);
    let (mut udev, mut signal) = create_dbus_handlers(engine_clone, receiver, tree_channel).await?;

    let pools = engine.pools().await;
    for (pool_name, pool_uuid, pool) in pools.iter() {
        udev.register_pool(pool_name, *pool_uuid, pool).await;
    }
    info!("D-Bus API is available");

    let mut signal_handle = task::spawn(async move {
        if let Err(e) = signal.process_dbus_actions().await {
            error!(
                "Failed to process D-Bus object path addition or removal: {e}; \
                exiting D-Bus thread",
            );
        }
    });
    let mut udev_handle = task::spawn(async move {
        loop {
            trace!("Starting D-Bus udev event handling");
            if let Err(e) = udev.handle_udev_event().await {
                error!(
                    "Failed to process udev event in the D-Bus layer: {e}; exiting D-Bus thread",
                );
                return;
            }
            trace!("Finished D-Bus udev event handling");
        }
    });

    select! {
        res = &mut signal_handle => {
            error!("The signal handling thread exited...");
            res.map_err(StratisError::from)
        }
        res = &mut udev_handle => {
            error!("The udev processing thread exited...");
            res.map_err(StratisError::from)
        }
    }
}

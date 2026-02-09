// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Conditionally compiled support for a D-Bus interface.

// Allow new_without_default lint, because otherwise it would be necessary
// to implement Default twice, one implementation for the supported version
// and one for the unsupported version. Also, Default is not really a
// helpful concept here.

use std::sync::Arc;

use tokio::{select, task};

use crate::{
    dbus::UdevHandler,
    engine::Engine,
    stratis::{StratisError, StratisResult},
};

/// Set up the cooperating D-Bus threads.
pub async fn setup(engine: Arc<dyn Engine>, mut udev: UdevHandler) -> StratisResult<()> {
    {
        let pools = engine.pools().await;
        for (_, pool_uuid, _) in pools.iter() {
            if let Err(e) = udev.register_pool(*pool_uuid).await {
                warn!("Failed to register existing pools: {e}");
            }
        }
    }
    info!("D-Bus API is available");

    let mut udev_handle = task::spawn(async move {
        loop {
            trace!("Starting D-Bus udev event handling");
            if let Err(e) = udev.process_udev_events().await {
                warn!("Failed to handle udev events: {e}");
                break;
            }
            trace!("Finished D-Bus udev event handling");
        }
    });

    select! {
        res = &mut udev_handle => {
            error!("The udev processing thread exited...");
            res.map_err(StratisError::from)
        }
    }
}

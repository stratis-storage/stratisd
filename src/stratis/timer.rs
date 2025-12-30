// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{sync::Arc, time::Duration};

#[cfg(feature = "dbus_enabled")]
use tokio::sync::RwLock;
use tokio::{task::spawn, time::sleep};
#[cfg(feature = "dbus_enabled")]
use zbus::Connection;

#[cfg(feature = "dbus_enabled")]
use crate::{
    dbus::{send_fs_background_signals, send_pool_background_signals, Manager},
    engine::Lockable,
};
use crate::{engine::Engine, stratis::errors::StratisResult};

/// Runs checks on thin pool usage and filesystem usage to determine whether either
/// need to be extended.
async fn check_pool_and_fs(
    #[cfg(feature = "dbus_enabled")] connection: Arc<Connection>,
    #[cfg(feature = "dbus_enabled")] manager: Lockable<Arc<RwLock<Manager>>>,
    engine: Arc<dyn Engine>,
) {
    async fn process_checks(
        #[cfg(feature = "dbus_enabled")] connection: &Arc<Connection>,
        #[cfg(feature = "dbus_enabled")] manager: Lockable<Arc<RwLock<Manager>>>,
        engine: &Arc<dyn Engine>,
    ) -> StratisResult<()> {
        #[cfg(any(feature = "min", not(feature = "dbus_enabled")))]
        {
            let _ = engine.pool_evented(None).await;
            let _ = engine.fs_evented(None).await;
        }
        #[cfg(feature = "dbus_enabled")]
        {
            let pool_diffs = engine.pool_evented(None).await;
            send_pool_background_signals(manager.clone(), connection, pool_diffs).await;
            let fs_diffs = engine.fs_evented(None).await;
            send_fs_background_signals(manager, connection, fs_diffs).await;
        }
        Ok(())
    }

    loop {
        trace!("Starting timed pool and filesystem checks");
        if let Err(e) = process_checks(
            #[cfg(feature = "dbus_enabled")]
            &connection,
            #[cfg(feature = "dbus_enabled")]
            manager.clone(),
            &engine,
        )
        .await
        {
            warn!("Failed to handle timed pool and filesystem checks: {e}");
        }
        trace!("Timed pool and filesystem checks finished");
        sleep(Duration::from_secs(10)).await;
    }
}

/// Run all timed background tasks.
///
/// Currently runs a timer to check thin pool and filesystem usage.
pub async fn run_timers(
    #[cfg(feature = "dbus_enabled")] connection: Arc<Connection>,
    #[cfg(feature = "dbus_enabled")] manager: Lockable<Arc<RwLock<Manager>>>,
    engine: Arc<dyn Engine>,
) -> StratisResult<()> {
    spawn(check_pool_and_fs(
        #[cfg(feature = "dbus_enabled")]
        connection,
        #[cfg(feature = "dbus_enabled")]
        manager,
        engine,
    ))
    .await?;
    Ok(())
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::time::Duration;

#[cfg(feature = "dbus_enabled")]
use tokio::sync::mpsc::UnboundedSender;
use tokio::{task::spawn, time::sleep};

#[cfg(feature = "dbus_enabled")]
use crate::dbus_api::DbusAction;
use crate::{
    engine::{Engine, LockableEngine},
    stratis::errors::StratisResult,
};

/// Runs checks on thin pool usage and filesystem usage to determine whether either
/// need to be extended.
async fn check_pool_and_fs<E>(
    engine: LockableEngine<E>,
    #[cfg(feature = "dbus_enabled")] sender: UnboundedSender<DbusAction<E>>,
) -> StratisResult<()>
where
    E: Engine,
{
    loop {
        sleep(Duration::from_secs(10)).await;
        let mut lock = engine.lock().await;
        // Return value currently not needed
        #[cfg(not(feature = "dbus_enabled"))]
        {
            let _ = lock.pool_evented(None)?;
            let _ = lock.fs_evented(None)?;
        }
        #[cfg(feature = "dbus_enabled")]
        {
            let pool_diffs = lock.pool_evented(None)?;
            for action in DbusAction::from_pool_diffs(pool_diffs) {
                if let Err(e) = sender.send(action) {
                    warn!(
                        "Failed to update D-Bus API with information on changed properties: {}",
                        e
                    );
                }
            }
            let fs_diffs = lock.fs_evented(None)?;
            for action in DbusAction::from_fs_diffs(fs_diffs) {
                if let Err(e) = sender.send(action) {
                    warn!(
                        "Failed to update D-Bus API with information on changed properties: {}",
                        e
                    );
                }
            }
        }
    }
}

/// Run all timed background tasks.
///
/// Currently runs a timer to check thin pool and filesystem usage.
pub async fn run_timers<E>(
    engine: LockableEngine<E>,
    #[cfg(feature = "dbus_enabled")] sender: UnboundedSender<DbusAction<E>>,
) -> StratisResult<()>
where
    E: 'static + Engine,
{
    spawn(check_pool_and_fs(
        engine,
        #[cfg(feature = "dbus_enabled")]
        sender,
    ))
    .await??;
    Ok(())
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{sync::Arc, time::Duration};

#[cfg(feature = "dbus_enabled")]
use tokio::sync::mpsc::UnboundedSender;
use tokio::{task::spawn, time::sleep};

#[cfg(feature = "dbus_enabled")]
use crate::dbus_api::DbusAction;
use crate::{engine::Engine, stratis::errors::StratisResult};

/// Runs checks on thin pool usage and filesystem usage to determine whether either
/// need to be extended.
async fn check_pool_and_fs<E>(
    engine: Arc<E>,
    #[cfg(feature = "dbus_enabled")] sender: UnboundedSender<DbusAction<E>>,
) where
    E: Engine,
{
    async fn process_checks<E>(
        engine: &Arc<E>,
        #[cfg(feature = "dbus_enabled")] sender: &UnboundedSender<DbusAction<E>>,
    ) -> StratisResult<()>
    where
        E: Engine,
    {
        #[cfg(feature = "min")]
        {
            let _ = engine.pool_evented(None).await;
            let _ = engine.fs_evented(None).await;
        }
        #[cfg(feature = "dbus_enabled")]
        {
            let pool_diffs = engine.pool_evented(None).await;
            for action in DbusAction::from_pool_diffs(pool_diffs) {
                if let Err(e) = sender.send(action) {
                    warn!(
                        "Failed to update D-Bus API with information on changed properties: {}",
                        e
                    );
                }
            }
            let fs_diffs = engine.fs_evented(None).await;
            for action in DbusAction::from_fs_diffs(fs_diffs) {
                if let Err(e) = sender.send(action) {
                    warn!(
                        "Failed to update D-Bus API with information on changed properties: {}",
                        e
                    );
                }
            }
        }
        Ok(())
    }

    loop {
        if let Err(e) = process_checks(
            &engine,
            #[cfg(feature = "dbus_enabled")]
            &sender,
        )
        .await
        {
            warn!("Failed to handle timed pool and filesystem checks: {}", e);
        }
        sleep(Duration::from_secs(10)).await;
    }
}

/// Run all timed background tasks.
///
/// Currently runs a timer to check thin pool and filesystem usage.
pub async fn run_timers<E>(
    engine: Arc<E>,
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
    .await?;
    Ok(())
}

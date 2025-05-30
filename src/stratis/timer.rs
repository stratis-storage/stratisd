// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{path::Path, sync::Arc, time::Duration};

use notify::{EventHandler, RecommendedWatcher, Watcher};
#[cfg(feature = "dbus_enabled")]
use tokio::sync::mpsc::UnboundedSender;
use tokio::{fs::File, io::AsyncReadExt, join, select, sync::watch, task::spawn, time::sleep};

#[cfg(feature = "dbus_enabled")]
use crate::dbus_api::DbusAction;
use crate::{
    engine::{get_persistent_keyring, Engine},
    stratis::{errors::StratisResult, StratisError},
};

const KEY_EXPIRY_PATH: &str = "/proc/sys/kernel/keys/persistent_keyring_expiry";

/// Runs checks on thin pool usage and filesystem usage to determine whether either
/// need to be extended.
async fn check_pool_and_fs(
    engine: Arc<dyn Engine>,
    #[cfg(feature = "dbus_enabled")] sender: UnboundedSender<DbusAction>,
) {
    async fn process_checks(
        engine: &Arc<dyn Engine>,
        #[cfg(feature = "dbus_enabled")] sender: &UnboundedSender<DbusAction>,
    ) -> StratisResult<()> {
        #[cfg(any(feature = "min", not(feature = "dbus_enabled")))]
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
        trace!("Starting timed pool and filesystem checks");
        if let Err(e) = process_checks(
            &engine,
            #[cfg(feature = "dbus_enabled")]
            &sender,
        )
        .await
        {
            warn!("Failed to handle timed pool and filesystem checks: {}", e);
        }
        trace!("Timed pool and filesystem checks finished");
        sleep(Duration::from_secs(10)).await;
    }
}

pub struct TokioEventHandler(watch::Sender<Option<notify::Result<notify::Event>>>);

impl EventHandler for TokioEventHandler {
    fn handle_event(&mut self, event: notify::Result<notify::Event>) {
        match event {
            Ok(e) => {
                if let notify::EventKind::Modify(_) = e.kind {
                    if let Err(e) = self.0.send(Some(Ok(e))) {
                        warn!("Failed to send inotify event to processing thread: {e}");
                    }
                }
            }
            Err(e) => {
                if let Err(e) = self.0.send(Some(Err(e))) {
                    warn!("Failed to send inotify event to processing thread: {e}");
                }
            }
        }
    }
}

/// Periodically refresh the keyring to ensure the keys are never garbage collected.
async fn refresh_keyring() {
    async fn process_timeout() -> StratisResult<u64> {
        let mut file = File::open("/proc/sys/kernel/keys/persistent_keyring_expiry").await?;
        let mut number = String::new();
        file.read_to_string(&mut number).await?;
        let trimmed = number.trim();
        trimmed
            .parse::<u64>()
            .map_err(|e| StratisError::Msg(e.to_string()))
    }

    async fn process_checks() -> StratisResult<()> {
        get_persistent_keyring().map(|_| ())
    }

    fn initialize_inotify(
        tx: watch::Sender<Option<notify::Result<notify::Event>>>,
    ) -> notify::Result<RecommendedWatcher> {
        let mut watcher = notify::recommended_watcher(TokioEventHandler(tx))?;
        watcher.watch(
            Path::new(KEY_EXPIRY_PATH),
            notify::RecursiveMode::NonRecursive,
        )?;
        Ok(watcher)
    }

    let (tx, mut rx) = watch::channel::<Option<notify::Result<notify::Event>>>(None);
    let _watcher = match initialize_inotify(tx) {
        Ok(w) => Some(w),
        Err(e) => {
            warn!("Failed to initialize inotify: {e}; keyring expiry may not be accurately tracked in stratisd");
            None
        }
    };

    loop {
        trace!("Starting timed keyring refresh");
        let timeout = match process_timeout().await {
            Ok(t) => t,
            Err(e) => {
                warn!("Failed to get timeout for keyring refresh: {e}");
                continue;
            }
        };
        if let Err(e) = process_checks().await {
            warn!("Failed to refresh keyring: {e}");
        }
        trace!("Timed keyring refresh finished");
        let timeout_adjusted = timeout.saturating_sub(3600);
        trace!("Sleeping for {} seconds", timeout_adjusted);
        select! {
            _ = sleep(Duration::from_secs(timeout_adjusted)) => {}
            res = rx.changed() => {
                if let Err(e) = res {
                    warn!("Failed to wait for inotify values: {e}")
                } else if let Some(Err(ref e)) = *rx.borrow() {
                    warn!("Inotify failure: {e}")
                }
            }
        }
    }
}

/// Run all timed background tasks.
///
/// Currently runs a timer to check thin pool and filesystem usage.
pub async fn run_timers(
    engine: Arc<dyn Engine>,
    #[cfg(feature = "dbus_enabled")] sender: UnboundedSender<DbusAction>,
) -> StratisResult<()> {
    let (res1, res2) = join!(
        spawn(check_pool_and_fs(
            engine,
            #[cfg(feature = "dbus_enabled")]
            sender,
        )),
        spawn(refresh_keyring())
    );
    res1?;
    res2?;
    Ok(())
}

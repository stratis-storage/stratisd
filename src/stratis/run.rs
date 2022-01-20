// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Main loop

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use tokio::{
    runtime::Builder,
    select, signal,
    sync::{broadcast::channel, mpsc::unbounded_channel},
    task,
};

#[cfg(feature = "dbus_enabled")]
use crate::dbus_api::DbusAction;
use crate::{
    engine::{Engine, SimEngine, StratEngine, UdevEngineEvent},
    stratis::{
        dm::dm_event_thread, errors::StratisResult, ipc_support::setup, stratis::VERSION,
        timer::run_timers, udev_monitor::udev_thread,
    },
};

// Waits for SIGINT. If received, sends true to all blocking calls in blocking
// threads which will then terminate.
async fn signal_thread() -> StratisResult<()> {
    task::spawn(async {
        if let Err(e) = signal::ctrl_c().await {
            error!("Failure while listening for signals: {}", e);
        }
    })
    .await?;

    Ok(())
}

/// Set up all sorts of signal and event handling mechanisms.
/// Initialize the engine and keep it running until a signal is received
/// or a fatal error is encountered.
/// If sim is true, start the sim engine rather than the real engine.
/// Always check for devicemapper context.
pub fn run(sim: bool) -> StratisResult<()> {
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .thread_name_fn(|| {
            static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
            let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
            format!("stratis-wt-{}", id)
        })
        .on_thread_start(|| {
            static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
            let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
            trace!("{}: thread started", id)
        })
        .on_thread_stop(|| {
            static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
            let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
            trace!("{}: thread finished", id)
        })
        .build()?;
    runtime.block_on(async move {
        async fn start_threads<E>(engine: Arc<E>, sim: bool) -> StratisResult<()> where E: 'static + Engine {
            let (trigger, should_exit) = channel(1);
            let (udev_sender, udev_receiver) = unbounded_channel::<UdevEngineEvent>();
            #[cfg(feature = "dbus_enabled")]
            let (dbus_sender, dbus_receiver) = unbounded_channel::<DbusAction<E>>();

            let join_udev = udev_thread(udev_sender, should_exit);
            let join_ipc = setup(
                engine.clone(),
                udev_receiver,
                #[cfg(feature = "dbus_enabled")]
                trigger.clone(),
                #[cfg(feature = "dbus_enabled")]
                (dbus_sender.clone(), dbus_receiver),
            );
            let join_signal = signal_thread();
            let join_dm = dm_event_thread(
                if sim {
                    None
                } else {
                    Some(Arc::clone(&engine))
                },
                #[cfg(feature = "dbus_enabled")]
                dbus_sender.clone(),
            );
            let join_timer = run_timers(
                engine,
                #[cfg(feature = "dbus_enabled")]
                dbus_sender,
            );

            select! {
                res = join_udev => {
                    if let Err(e) = res {
                        error!("The udev thread exited with an error: {}; shutting down stratisd...", e);
                        return Err(e);
                    } else {
                        info!("The udev thread exited; shutting down stratisd...");
                    }
                }
                res = join_ipc => {
                    if let Err(e) = res {
                        error!("The IPC thread exited with an error: {}; shutting down stratisd...", e);
                    } else {
                        info!("The IPC thread exited; shutting down stratisd...");
                    }
                },
                Err(e) = join_dm => {
                    error!("The devicemapper thread exited with an error: {}; shutting down stratisd...", e);
                    return Err(e);
                },
                res = join_timer => {
                    if let Err(e) = res {
                        error!("The timer thread exited with an error: {}; shutting down stratisd...", e);
                        return Err(e);
                    } else {
                        info!("The timer thread exited; shutting down stratisd...");
                    }
                },
                _ = join_signal => {
                    info!("Caught SIGINT; exiting...");
                },
            }

            if let Err(e) = trigger.send(()) {
                warn!("Failed to notify blocking stratisd threads to shut down: {}", e);
            }

            Ok(())
        }

        info!("stratis daemon version {} started", VERSION);
        if sim {
            info!("Using SimEngine");
            start_threads(Arc::new(SimEngine::default()), sim).await
        } else {
            info!("Using StratEngine");
            start_threads(
                Arc::new(match StratEngine::initialize() {
                    Ok(engine) => engine,
                    Err(e) => {
                        error!("Failed to start up stratisd engine: {}; exiting", e);
                        return Err(e);
                    }
                }),
                sim
            ).await
        }
    })?;
    Ok(())
}

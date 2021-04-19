// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Main loop

use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

use tokio::{runtime::Builder, select, signal, sync::mpsc::unbounded_channel, task};

use crate::{
    engine::{Engine, Locked, SimEngine, StratEngine, UdevEngineEvent},
    stratis::{
        dm::dm_event_thread, errors::StratisResult, ipc_support::setup, stratis::VERSION,
        udev_monitor::udev_thread,
    },
};

// Waits for SIGINT. If received, sets should_exit to true.
async fn signal_thread(should_exit: Arc<AtomicBool>) {
    if let Err(e) = signal::ctrl_c().await {
        error!("Failure while listening for signals: {}", e);
    }
    should_exit.store(true, Ordering::Relaxed);
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
            debug!("{}: thread started", id)
        })
        .on_thread_stop(|| {
            static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
            let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
            debug!("{}: thread finished", id)
        })
        .build()?;
    runtime.block_on(async move {
        let engine: Locked<dyn Engine> = {
            info!("stratis daemon version {} started", VERSION);
            if sim {
                info!("Using SimEngine");
                Locked::new(SimEngine::default()).into_dyn_engine()
            } else {
                info!("Using StratEngine");
                Locked::new(match StratEngine::initialize() {
                    Ok(engine) => engine,
                    Err(e) => {
                        error!("Failed to start up stratisd engine: {}; exiting", e);
                        return Err(e);
                    }
                }).into_dyn_engine()
            }
        };

        let should_exit = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = unbounded_channel::<UdevEngineEvent>();

        let udev_arc_clone = Arc::clone(&should_exit);
        let join_udev = task::spawn_blocking(move || udev_thread(sender, udev_arc_clone));
        let join_ipc = task::spawn(setup(engine.clone(), receiver));
        let join_signal = task::spawn(signal_thread(Arc::clone(&should_exit)));
        let join_dm = task::spawn(dm_event_thread(if sim {
            None
        } else {
            Some(engine.clone())
        }));

        select! {
            res = join_udev => {
                if let Ok(Err(e)) = res {
                    error!("The udev thread exited with an error: {}; shutting down stratisd...", e);
                    return Err(e);
                } else {
                    error!("The udev thread exited; shutting down stratisd...");
                }
            }
            res = join_ipc => {
                if let Ok(Err(e)) = res {
                    error!("The IPC thread exited with an error: {}; shutting down stratisd...", e);
                    return Err(e);
                } else {
                    error!("The IPC thread exited; shutting down stratisd...");
                }
            }
            Ok(Err(e)) = join_dm => {
                error!("The devicemapper thread exited with an error: {}; shutting down stratisd...", e);
                return Err(e);
            }
            _ = join_signal => {
                info!("Caught SIGINT; exiting...");
            }
        }
        should_exit.store(true, Ordering::Relaxed);
        Ok(())
    })?;
    Ok(())
}

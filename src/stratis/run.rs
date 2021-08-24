// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Main loop

use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::{
    runtime::Builder,
    select, signal,
    sync::{broadcast::channel, mpsc::unbounded_channel},
    task,
};

use crate::{
    engine::{Lockable, LockableEngine, SimEngine, StratEngine, UdevEngineEvent},
    stratis::{
        dm::dm_event_thread, errors::StratisResult, ipc_support::setup, stratis::VERSION,
        udev_monitor::udev_thread,
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
            debug!("{}: thread started", id)
        })
        .on_thread_stop(|| {
            static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
            let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
            debug!("{}: thread finished", id)
        })
        .build()?;
    runtime.block_on(async move {
        let engine: LockableEngine = {
            info!("stratis daemon version {} started", VERSION);
            if sim {
                info!("Using SimEngine");
                Lockable::new_exclusive(SimEngine::default())
            } else {
                info!("Using StratEngine");
                Lockable::new_exclusive(match StratEngine::initialize() {
                    Ok(engine) => engine,
                    Err(e) => {
                        error!("Failed to start up stratisd engine: {}; exiting", e);
                        return Err(e);
                    }
                })
            }
        };

        let (trigger, should_exit) = channel(1);
        let (sender, receiver) = unbounded_channel::<UdevEngineEvent>();

        let join_udev = udev_thread(sender, should_exit);
        let join_ipc = setup(engine.clone(), receiver, trigger.clone());
        let join_signal = signal_thread();
        let join_dm = dm_event_thread(if sim {
            None
        } else {
            Some(engine.clone())
        });

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
                    return Err(e);
                } else {
                    info!("The IPC thread exited; shutting down stratisd...");
                }
            }
            Err(e) = join_dm => {
                error!("The devicemapper thread exited with an error: {}; shutting down stratisd...", e);
                return Err(e);
            }
            _ = join_signal => {
                info!("Caught SIGINT; exiting...");
            }
        }
        if let Err(e) = trigger.send(()) {
            warn!("Failed to notify blocking stratisd threads to shut down: {}", e);
        }
        Ok(())
    })?;
    Ok(())
}

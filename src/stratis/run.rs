// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Main loop

use std::{
    os::unix::io::AsRawFd,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use futures::executor::block_on;
use nix::poll::{poll, PollFd, PollFlags};
use tokio::{
    runtime::Runtime,
    select, signal,
    sync::{
        mpsc::{channel, Sender},
        Mutex,
    },
    task,
};
use tokio_stream::StreamExt;

use crate::{
    engine::{Engine, SimEngine, StratEngine, UdevEngineEvent},
    stratis::{
        dm::DmFd, errors::StratisResult, ipc_support::setup, stratis::VERSION,
        udev_monitor::UdevMonitor,
    },
};

// Poll for udev events.
// Check for exit condition and return if true.
fn udev_thread(sender: Sender<UdevEngineEvent>, should_exit: Arc<AtomicBool>) -> StratisResult<()> {
    let context = libudev::Context::new()?;
    let mut udev = UdevMonitor::create(&context)?;

    let mut pollers = [PollFd::new(udev.as_raw_fd(), PollFlags::POLLIN)];
    loop {
        match poll(&mut pollers, 100)? {
            0 => {
                if should_exit.load(Ordering::Relaxed) {
                    info!("udev thread was notified to exit");
                    return Ok(());
                }
            }
            _ => {
                if let Some(ref e) = udev.poll() {
                    if let Err(e) = block_on(sender.send(UdevEngineEvent::from(e))) {
                        warn!(
                            "udev event could not be sent to engine thread: {}; the \
                            engine was not notified of this udev event",
                            e,
                        );
                    }
                }
            }
        }
    }
}

async fn signal_thread(should_exit: Arc<AtomicBool>) {
    if let Err(e) = signal::ctrl_c().await {
        error!("Failure while listening for signals: {}", e);
    }
    should_exit.store(true, Ordering::Relaxed);
}

async fn dm_event_thread(engine: Option<Arc<Mutex<dyn Engine>>>) -> StratisResult<()> {
    match engine {
        Some(e) => {
            let mut dm_fd_opt = DmFd::new(e)?;
            loop {
                dm_fd_opt.next().await;
            }
        }
        None => Ok(()),
    }
}

/// Set up all sorts of signal and event handling mechanisms.
/// Initialize the engine and keep it running until a signal is received
/// or a fatal error is encountered.
/// If sim is true, start the sim engine rather than the real engine.
/// Always check for devicemapper context.
pub fn run(sim: bool) -> StratisResult<()> {
    let runtime = Runtime::new()?;
    runtime.block_on(async move {
        let engine: Arc<Mutex<dyn Engine>> = {
            info!("stratis daemon version {} started", VERSION);
            if sim {
                info!("Using SimEngine");
                Arc::new(Mutex::new(SimEngine::default()))
            } else {
                info!("Using StratEngine");
                Arc::new(Mutex::new(match StratEngine::initialize() {
                    Ok(engine) => engine,
                    Err(e) => {
                        error!("Failed to start up stratisd engine: {}; exiting", e);
                        return;
                    }
                }))
            }
        };

        let should_exit = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = channel::<UdevEngineEvent>(1024);

        let udev_arc_clone = Arc::clone(&should_exit);
        let join_udev = task::spawn_blocking(move || udev_thread(sender, udev_arc_clone));
        let join_ipc = task::spawn(setup(Arc::clone(&engine), receiver));
        let join_signal = task::spawn(signal_thread(Arc::clone(&should_exit)));
        let join_dm = task::spawn(dm_event_thread(if sim {
            None
        } else {
            Some(Arc::clone(&engine))
        }));

        select! {
            res = join_udev => {
                if let Ok(Err(e)) = res {
                    error!("The udev thread exited with an error: {}; shutting down stratisd...", e);
                } else {
                    error!("The udev thread exited; shutting down stratisd...");
                }
            }
            res = join_ipc => {
                if let Ok(Err(e)) = res {
                    error!("The IPC thread exited with an error: {}; shutting down stratisd...", e);
                } else {
                    error!("The IPC thread exited; shutting down stratisd...");
                }
            }
            Ok(Err(e)) = join_dm => {
                error!("The devicemapper thread exited with an error: {}; shutting down stratisd...", e);
            }
            _ = join_signal => {
                info!("Caught SIGINT; exiting...");
            }
        }
        should_exit.store(true, Ordering::Relaxed);
    });
    Ok(())
}

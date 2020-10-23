// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Main loop

use std::{
    os::unix::io::AsRawFd,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    },
};

use mio::unix::EventedFd;
use tokio::{runtime::Runtime, select, signal, task};

use crate::{
    engine::{Engine, SimEngine, StratEngine, UdevEngineEvent},
    stratis::{errors::StratisResult, stratis::VERSION, udev_monitor::UdevMonitor},
};

fn udev_thread(sender: Sender<UdevEngineEvent>) {
    let context = match libudev::Context::new() {
        Ok(ctxt) => ctxt,
        Err(e) => {
            error!("Failed to acquire udev context: {}; exiting udev thread", e);
            return;
        }
    };
    let mut udev = match UdevMonitor::create(&context) {
        Ok(udev) => udev,
        Err(e) => {
            error!("Failed to set up udev thread: {}; exiting udev thread", e);
            return;
        }
    };

    let fd = udev.as_raw_fd();
    let evented_fd = EventedFd(&fd);
    if let Err(e) = udev.register(&evented_fd) {
        error!(
            "Failed to register udev socket for polling: {}; exiting udev thread",
            e
        );
        return;
    }

    while let Some(ref event) = udev.poll() {
        match event {
            Ok(e) => {
                let _ = sender.send(UdevEngineEvent::from(e)).map_err(|e| {
                    warn!(
                        "udev event could not be sent to engine thread: {}; the \
                            engine was not notified of this udev event",
                        e,
                    );
                });
            }
            Err(e) => {
                error!(
                    "Failed to poll udev for an event: {}; exiting udev thread",
                    e
                );
                return;
            }
        }
    }
}

fn ipc_thread(_engine: Arc<Mutex<dyn Engine>>, _receiver: Receiver<UdevEngineEvent>) {}

async fn signal_thread() {
    if let Err(e) = signal::ctrl_c().await {
        error!("Failure while listening for signals: {}", e);
    }
}

/// Set up all sorts of signal and event handling mechanisms.
/// Initialize the engine and keep it running until a signal is received
/// or a fatal error is encountered. Dump log entries on specified signal
/// via buff_log.
/// If sim is true, start the sim engine rather than the real engine.
pub fn run(sim: bool) -> StratisResult<()> {
    let mut runtime = Runtime::new()?;
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

        let (sender, receiver) = channel::<UdevEngineEvent>();

        let mut join_udev = task::spawn_blocking(move || udev_thread(sender));
        let engine_ref = Arc::clone(&engine);
        let mut join_ipc = task::spawn_blocking(move || ipc_thread(engine_ref, receiver));
        let mut join_signal = task::spawn(signal_thread());

        select! {
            _ = &mut join_udev => {
                error!("The udev thread exited; shutting down stratisd...");
            }
            _ = &mut join_ipc => {
                error!("The IPC thread exited; shutting down stratisd...");
            }
            _ = &mut join_signal => {
                info!("Caught SIGINT; exiting...");
            }
        }
    });
    Ok(())

    // /*
    //fds is a Vec of libc::pollfd structs. Ideally, it would be possible
    //to use the higher level nix crate to handle polling. If this were possible,
    //then the Vec would be one of nix::poll::PollFds and this would be more
    //rustic. Unfortunately, the rust D-Bus library requires an explicit file
    //descriptor to be passed as an argument to Connection::watch_handle(),
    //and the explicit file descriptor can not be extracted from the PollFd
    //struct. So, at this time, sticking with libc is less complex than
    //converting to using nix, because if using nix, the file descriptor would
    //have to be maintained in the Vec as well as the PollFd struct.
    // */
    //let mut fds = Vec::new();

    //fds.push(libc::pollfd {
    //    fd: udev_monitor.as_raw_fd(),
    //    revents: 0,
    //    events: libc::POLLIN,
    //});

    //let eventable = engine.borrow().get_eventable();

    //if let Some(evt) = eventable {
    //    fds.push(libc::pollfd {
    //        fd: evt.get_pollable_fd(),
    //        revents: 0,
    //        events: libc::POLLIN,
    //    });
    //};

    //let dbus_client_index_start = if eventable.is_some() {
    //    FD_INDEX_ENGINE + 1
    //} else {
    //    FD_INDEX_ENGINE
    //};

    //loop {
    //    if fds[FD_INDEX_UDEV].revents != 0 {
    //        udev_monitor.handle_events(&mut *engine.borrow_mut(), &mut dbus_support)
    //    }

    //    if let Some(evt) = eventable {
    //        if fds[FD_INDEX_ENGINE].revents != 0 {
    //            evt.clear_event()?;
    //            engine.borrow_mut().evented()?;
    //        }
    //    }

    //    dbus_support.process(&mut fds, dbus_client_index_start);

    //    process_poll(&mut fds)?;
    //}
}

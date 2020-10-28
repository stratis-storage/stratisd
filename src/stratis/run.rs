// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Main loop

use std::{
    os::unix::io::AsRawFd,
    sync::{
        mpsc::{channel, Sender},
        Arc, Mutex,
    },
    time::Duration,
};

use mio::{unix::EventedFd, Events, Poll, PollOpt, Ready, Token};
use tokio::{runtime::Runtime, select, signal, task};

use crate::{
    engine::{Engine, SimEngine, StratEngine, UdevEngineEvent},
    stratis::{
        errors::StratisResult, ipc_support::setup, stratis::VERSION, udev_monitor::UdevMonitor,
    },
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

async fn signal_thread() {
    if let Err(e) = signal::ctrl_c().await {
        error!("Failure while listening for signals: {}", e);
    }
}

fn dm_event_thread(engine: Arc<Mutex<dyn Engine>>) {
    fn dm_event_thread_res(engine: Arc<Mutex<dyn Engine>>) -> StratisResult<()> {
        let fd = {
            let lock = engine.lock()?;
            match (*lock).get_eventable() {
                Some(evt) => evt.get_pollable_fd(),
                None => return Ok(()),
            }
        };
        let evented_fd = EventedFd(&fd);
        let poll = Poll::new()?;
        poll.register(&evented_fd, Token(0), Ready::readable(), PollOpt::level())?;

        let mut events = Events::with_capacity(1);
        loop {
            poll.poll(&mut events, None)?;
            for event in &events {
                if event.token() == Token(0) && event.readiness() == Ready::readable() {
                    let mut lock = engine.lock()?;
                    if let Some(evt) = (*lock).get_eventable() {
                        evt.clear_event()?;
                        (*lock).evented()?;
                    }
                }
            }
        }
    }

    if let Err(e) = dm_event_thread_res(engine) {
        error!("devicemapper event thread failed: {}", e);
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
        let mut join_ipc = task::spawn(setup(Arc::clone(&engine), receiver));
        let mut join_signal = task::spawn(signal_thread());
        let mut join_dm = task::spawn_blocking(move || dm_event_thread(engine));

        select! {
            _ = &mut join_udev => {
                error!("The udev thread exited; shutting down stratisd...");
            }
            _ = &mut join_ipc => {
                error!("The IPC thread exited; shutting down stratisd...");
            }
            _ = &mut join_dm => {
                error!("The devicemapper thread exiting; shutting down stratisd...");
            }
            _ = &mut join_signal => {
                info!("Caught SIGINT; exiting...");
            }
        }
    });
    runtime.shutdown_timeout(Duration::from_secs(0));
    Ok(())
}

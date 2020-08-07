// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Main loop

use std::{cell::RefCell, os::unix::io::AsRawFd, rc::Rc};

use nix::sys::signalfd::{signal, SfdFlags, SigSet, SignalFd};

use crate::{
    engine::{Engine, SimEngine, StratEngine},
    stratis::{
        dbus_support::MaybeDbusSupport,
        errors::{StratisError, StratisResult},
        stratis::VERSION,
        udev_monitor::UdevMonitor,
    },
};

// Process any pending signals, return true if SIGINT received.
// Return an error if there was an error reading the signal.
fn process_signal(sfd: &mut SignalFd) -> StratisResult<bool> {
    match sfd.read_signal() {
        // This is an unsafe conversion, but in this context that is
        // mostly harmless. A negative converted value, which is
        // virtually impossible, will not match any of the masked
        // values, and stratisd will panic and exit.
        Ok(Some(sig)) => match sig.ssi_signo as i32 {
            nix::libc::SIGINT => {
                info!("SIGINT received, exiting");
                Ok(true)
            }
            signo => {
                panic!("Caught an impossible signal {:?}", signo);
            }
        },
        // No signals waiting (SFD_NONBLOCK flag is set)
        Ok(None) => Ok(false),

        Err(err) => Err(err.into()),
    }
}

/// Handle blocking the event loop
fn process_poll(poll_timeout: i32, fds: &mut Vec<libc::pollfd>) -> StratisResult<()> {
    let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::c_ulong, poll_timeout) };

    // TODO: refine this behavior.
    // Different behaviors may be indicated, depending on the value of
    // errno when return value is -1.
    if r < 0 {
        return Err(StratisError::Error(format!(
            "poll command failed: number of fds: {}, timeout: {}",
            fds.len(),
            poll_timeout
        )));
    }
    Ok(())
}

/// Set up all sorts of signal and event handling mechanisms.
/// Initialize the engine and keep it running until a signal is received
/// or a fatal error is encountered. Dump log entries on specified signal
/// via buff_log.
/// If sim is true, start the sim engine rather than the real engine.
pub fn run(sim: bool) -> StratisResult<()> {
    let mut dbus_support = MaybeDbusSupport::new();

    // Setup a udev listener before initializing the engine. A device may
    // appear after the engine has processed the udev db, but before it has
    // completed initialization. Unless the udev event has been recorded, the
    // engine will miss the device.
    // This is especially important since stratisd must run during early boot.
    let context = libudev::Context::new()?;
    let mut udev_monitor = UdevMonitor::create(&context)?;

    let engine: Rc<RefCell<dyn Engine>> = {
        info!("stratis daemon version {} started", VERSION);
        if sim {
            info!("Using SimEngine");
            Rc::new(RefCell::new(SimEngine::default()))
        } else {
            info!("Using StratEngine");
            Rc::new(RefCell::new(StratEngine::initialize()?))
        }
    };

    /*
    The file descriptor array indexes are:

    0   == Always udev fd index
    1   == SIGNAL FD index
    2   == engine index if eventable
    2/3 == Start of dbus client file descriptor(s)
            * 2 if engine is not eventable
            * else 3
    */
    const FD_INDEX_UDEV: usize = 0;
    const FD_INDEX_SIGNALFD: usize = 1;
    const FD_INDEX_ENGINE: usize = 2;

    /*
    fds is a Vec of libc::pollfd structs. Ideally, it would be possible
    to use the higher level nix crate to handle polling. If this were possible,
    then the Vec would be one of nix::poll::PollFds and this would be more
    rustic. Unfortunately, the rust D-Bus library requires an explicit file
    descriptor to be passed as an argument to Connection::watch_handle(),
    and the explicit file descriptor can not be extracted from the PollFd
    struct. So, at this time, sticking with libc is less complex than
    converting to using nix, because if using nix, the file descriptor would
    have to be maintained in the Vec as well as the PollFd struct.
    */
    let mut fds = Vec::new();

    fds.push(libc::pollfd {
        fd: udev_monitor.as_raw_fd(),
        revents: 0,
        events: libc::POLLIN,
    });

    // Signals can be queued up on this file descriptor
    let mut sfd = {
        let mut mask = SigSet::empty();
        mask.add(signal::SIGINT);
        mask.thread_block()?;
        SignalFd::with_flags(&mask, SfdFlags::SFD_NONBLOCK)?
    };

    fds.push(libc::pollfd {
        fd: sfd.as_raw_fd(),
        revents: 0,
        events: libc::POLLIN,
    });

    let eventable = engine.borrow().get_eventable();

    if let Some(evt) = eventable {
        fds.push(libc::pollfd {
            fd: evt.get_pollable_fd(),
            revents: 0,
            events: libc::POLLIN,
        });
    };

    let dbus_client_index_start = if eventable.is_some() {
        FD_INDEX_ENGINE + 1
    } else {
        FD_INDEX_ENGINE
    };

    loop {
        if fds[FD_INDEX_UDEV].revents != 0 {
            udev_monitor.handle_events(&mut *engine.borrow_mut(), &mut dbus_support)
        }

        if fds[FD_INDEX_SIGNALFD].revents != 0 {
            match process_signal(&mut sfd) {
                Ok(should_exit) => {
                    if should_exit {
                        return Ok(());
                    }
                }
                Err(e) => return Err(e),
            }
        }

        if let Some(evt) = eventable {
            if fds[FD_INDEX_ENGINE].revents != 0 {
                evt.clear_event()?;
                engine.borrow_mut().evented()?;
            }
        }

        dbus_support.process(&engine, &mut fds, dbus_client_index_start);

        process_poll(dbus_support.poll_timeout(), &mut fds)?;
    }
}

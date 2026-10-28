// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Support for monitoring udev events.

use std::os::unix::io::{AsFd, AsRawFd, BorrowedFd};

use libudev::Event;
use nix::poll::{poll, PollFd, PollFlags};
use tokio::{
    sync::{
        broadcast::{error::TryRecvError, Receiver},
        mpsc::UnboundedSender,
        oneshot,
    },
    task::spawn_blocking,
};

use crate::{
    engine::UdevEngineEvent,
    stratis::errors::{StratisError, StratisResult},
};

// Poll for udev events.
// Check for exit condition and return if true.
// Signals on ready once the monitor is listening. If setup fails, ready is
// dropped without a signal so that the receiver observes the failure.
pub async fn udev_thread(
    sender: UnboundedSender<UdevEngineEvent>,
    ready: oneshot::Sender<()>,
    mut should_exit: Receiver<()>,
) -> StratisResult<()> {
    spawn_blocking(move || {
        let context = libudev::Context::new()?;
        let mut udev = UdevMonitor::create(&context)?;

        // The receiver is gone only if the engine already failed to start,
        // in which case the error is reported there.
        let _ = ready.send(());

        loop {
            let mut pollers = [PollFd::new(udev.as_fd(), PollFlags::POLLIN)];

            match poll(&mut pollers, 100u8)? {
                0 => {
                    match should_exit.try_recv() {
                        Ok(()) => {
                            info!("udev thread was notified to exit");
                            return Ok(());
                        }
                        Err(TryRecvError::Closed | TryRecvError::Lagged(_)) => {
                            return Err(StratisError::Msg(
                                "udev processing thread can no longer be notified to exit; shutting down...".to_string()
                            ));
                        }
                        _ => (),
                    };
                }
                _ => {
                    if let Some(ref e) = udev.poll() {
                        if let Err(e) = sender.send(UdevEngineEvent::from(e)) {
                            warn!(
                                "udev event could not be sent to engine thread: {e}; the \
                                engine was not notified of this udev event",
                            );
                        }
                    }
                }
            }
        }
    })
    .await??;
    Ok(())
}

/// A facility for listening for and handling udev events that stratisd
/// considers interesting.
struct UdevMonitor {
    socket: libudev::MonitorSocket,
}

impl UdevMonitor {
    fn create(context: &libudev::Context) -> StratisResult<UdevMonitor> {
        let mut monitor = libudev::Monitor::new(context)?;
        monitor.match_subsystem("block")?;

        let socket = monitor.listen()?;

        Ok(UdevMonitor { socket })
    }

    pub fn poll(&mut self) -> Option<Event> {
        self.socket.receive_event()
    }
}

impl AsFd for UdevMonitor {
    fn as_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(self.socket.as_raw_fd()) }
    }
}

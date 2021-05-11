// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Support for monitoring udev events.

use std::os::unix::io::{AsRawFd, RawFd};

use libudev::Event;
use nix::poll::{poll, PollFd, PollFlags};
use tokio::sync::{broadcast::Receiver, mpsc::UnboundedSender};

use crate::{engine::UdevEngineEvent, stratis::errors::StratisResult};

// Poll for udev events.
// Check for exit condition and return if true.
pub fn udev_thread(
    sender: UnboundedSender<UdevEngineEvent>,
    mut should_exit: Receiver<bool>,
) -> StratisResult<()> {
    let context = libudev::Context::new()?;
    let mut udev = UdevMonitor::create(&context)?;

    let mut pollers = [PollFd::new(udev.as_raw_fd(), PollFlags::POLLIN)];
    loop {
        match poll(&mut pollers, 100)? {
            0 => {
                if let Ok(true) = should_exit.try_recv() {
                    info!("udev thread was notified to exit");
                    return Ok(());
                }
            }
            _ => {
                if let Some(ref e) = udev.poll() {
                    if let Err(e) = sender.send(UdevEngineEvent::from(e)) {
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

/// A facility for listening for and handling udev events that stratisd
/// considers interesting.
struct UdevMonitor<'a> {
    socket: libudev::MonitorSocket<'a>,
}

impl<'a> UdevMonitor<'a> {
    fn create(context: &'a libudev::Context) -> StratisResult<UdevMonitor<'a>> {
        let mut monitor = libudev::Monitor::new(&context)?;
        monitor.match_subsystem("block")?;

        let socket = monitor.listen()?;

        Ok(UdevMonitor { socket })
    }

    pub fn poll(&mut self) -> Option<Event<'a>> {
        self.socket.receive_event()
    }
}

impl<'a> AsRawFd for UdevMonitor<'a> {
    fn as_raw_fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }
}

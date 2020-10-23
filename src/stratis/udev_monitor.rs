// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Support for monitoring udev events.

use std::os::unix::io::{AsRawFd, RawFd};

use libudev::Event;
use mio::{unix::EventedFd, Events, Poll, PollOpt, Ready, Token};

use crate::stratis::errors::{StratisError, StratisResult};

/// A facility for listening for and handling udev events that stratisd
/// considers interesting.
pub struct UdevMonitor<'a> {
    socket: libudev::MonitorSocket<'a>,
    poll: Poll,
}

impl<'a> UdevMonitor<'a> {
    pub fn create(context: &'a libudev::Context) -> StratisResult<UdevMonitor<'a>> {
        let mut monitor = libudev::Monitor::new(&context)?;
        monitor.match_subsystem("block")?;

        let socket = monitor.listen()?;
        let poll = Poll::new()?;

        Ok(UdevMonitor { socket, poll })
    }

    pub fn register<'b>(&mut self, evented_fd: &'b EventedFd<'b>) -> StratisResult<()> {
        self.poll
            .register(evented_fd, Token(0), Ready::readable(), PollOpt::level())?;
        Ok(())
    }

    pub fn poll(&mut self) -> Option<StratisResult<Event<'a>>> {
        let mut events = Events::with_capacity(1);
        if let Err(e) = self.poll.poll(&mut events, None) {
            return Some(Err(StratisError::from(e)));
        }
        for event in &events {
            if event.token() == Token(0) && event.readiness() == Ready::readable() {
                return self.socket.receive_event().map(Ok);
            }
        }
        None
    }
}

impl<'a> AsRawFd for UdevMonitor<'a> {
    fn as_raw_fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }
}

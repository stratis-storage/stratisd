// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Support for monitoring udev events.

use std::os::unix::io::{AsRawFd, RawFd};

use libudev::Event;

use crate::stratis::errors::StratisResult;

/// A facility for listening for and handling udev events that stratisd
/// considers interesting.
pub struct UdevMonitor<'a> {
    socket: libudev::MonitorSocket<'a>,
}

impl<'a> UdevMonitor<'a> {
    pub fn create(context: &'a libudev::Context) -> StratisResult<UdevMonitor<'a>> {
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

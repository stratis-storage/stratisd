// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Support for monitoring udev events.

use std::os::unix::io::{AsRawFd, RawFd};

use either::Either;

use crate::{
    engine::Engine,
    stratis::{dbus_support::MaybeDbusSupport, errors::StratisResult},
};

/// A facility for listening for and handling udev events that stratisd
/// considers interesting.
pub struct UdevMonitor<'a> {
    socket: libudev::MonitorSocket<'a>,
}

impl<'a> UdevMonitor<'a> {
    pub fn create(context: &'a libudev::Context) -> StratisResult<UdevMonitor<'a>> {
        let mut monitor = libudev::Monitor::new(context)?;
        monitor.match_subsystem("block")?;

        Ok(UdevMonitor {
            socket: monitor.listen()?,
        })
    }

    pub fn as_raw_fd(&mut self) -> RawFd {
        self.socket.as_raw_fd()
    }

    /// Handle udev events.
    /// Check if a pool can be constructed and update engine and D-Bus layer
    /// data structures if so.
    pub fn handle_events(&mut self, engine: &mut dyn Engine, dbus_support: &mut MaybeDbusSupport) {
        while let Some(event) = self.socket.receive_event() {
            match engine.handle_event(&event) {
                Some(Either::Left((pool_name, pool_uuid, pool))) => {
                    dbus_support.register_pool(&pool_name, pool_uuid, pool);
                }
                Some(Either::Right((pool_uuid, device_set))) => {
                    dbus_support.register_device_set(pool_uuid, device_set);
                }
                None => {}
            }
        }
    }
}

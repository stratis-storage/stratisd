// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Conditionally compiled support for a D-Bus interface.

// Allow new_without_default lint, because otherwise it would be necessary
// to implement Default twice, one implementation for the supported version
// and one for the unsupported version. Also, Default is not really a
// helpful concept here.

use std::{cell::RefCell, rc::Rc};

use crate::engine::{Engine, Pool, PoolUuid};

#[cfg(feature = "dbus_enabled")]
use crate::{
    dbus_api::{DbusConnectionData, EventHandler},
    engine::{get_engine_listener_list_mut, Name},
};

pub struct MaybeDbusSupport {
    #[cfg(feature = "dbus_enabled")]
    handle: Option<DbusConnectionData>,
}

// If D-Bus compiled out, do very little.
#[cfg(not(feature = "dbus_enabled"))]
impl MaybeDbusSupport {
    #[allow(clippy::new_without_default)]
    pub fn new() -> MaybeDbusSupport {
        MaybeDbusSupport {}
    }

    pub fn process(
        &mut self,
        _engine: &Rc<RefCell<dyn Engine>>,
        _fds: &mut Vec<libc::pollfd>,
        _dbus_client_index_start: usize,
    ) {
    }

    pub fn register_pool(&mut self, _pool_name: Name, _pool_uuid: PoolUuid, _pool: &mut dyn Pool) {}

    pub fn poll_timeout(&self) -> i32 {
        // Non-DBus timeout is infinite
        -1
    }
}

#[cfg(feature = "dbus_enabled")]
impl MaybeDbusSupport {
    #[allow(clippy::new_without_default)]
    pub fn new() -> MaybeDbusSupport {
        MaybeDbusSupport { handle: None }
    }

    /// Connect to D-Bus and register pools, if not already connected.
    /// Return the connection, if made or already existing, otherwise, None.
    fn setup_connection(
        &mut self,
        engine: &Rc<RefCell<dyn Engine>>,
    ) -> Option<&mut DbusConnectionData> {
        if self.handle.is_none() {
            match DbusConnectionData::connect(Rc::clone(engine)) {
                Err(err) => {
                    warn!("D-Bus API is not available: {}", err);
                }
                Ok(mut handle) => {
                    info!("D-Bus API is available");
                    let event_handler = Box::new(EventHandler::new(Rc::clone(&handle.connection)));
                    get_engine_listener_list_mut().register_listener(event_handler);
                    // Register all the pools with dbus
                    for (pool_name, pool_uuid, pool) in engine.borrow_mut().pools_mut() {
                        handle.register_pool(pool_name, pool_uuid, pool)
                    }
                    self.handle = Some(handle);
                }
            }
        };
        self.handle.as_mut()
    }

    /// Handle any client dbus requests.
    pub fn process(
        &mut self,
        engine: &Rc<RefCell<dyn Engine>>,
        fds: &mut Vec<libc::pollfd>,
        dbus_client_index_start: usize,
    ) {
        if let Some(handle) = self.setup_connection(engine) {
            handle.handle(&fds[dbus_client_index_start..]);

            // Refresh list of dbus fds to poll for. This can change as
            // D-Bus clients come and go.
            fds.truncate(dbus_client_index_start);
            fds.extend(
                handle
                    .connection
                    .borrow()
                    .watch_fds()
                    .iter()
                    .map(|w| w.to_pollfd()),
            );
        }
    }

    pub fn register_pool(&mut self, pool_name: Name, pool_uuid: PoolUuid, pool: &mut dyn Pool) {
        if let Some(h) = self.handle.as_mut() {
            h.register_pool(pool_name, pool_uuid, pool)
        }
    }

    pub fn poll_timeout(&self) -> i32 {
        // If there is no D-Bus connection set timeout to 1 sec (1000 ms), so
        // that stratisd can periodically attempt to set up a connection.
        // If the connection is up, set the timeout to infinite; there is no
        // need to poll as events will be received.
        self.handle.as_ref().map_or(1000, |_| -1)
    }
}

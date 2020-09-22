// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Conditionally compiled support for a D-Bus interface.

// Allow new_without_default lint, because otherwise it would be necessary
// to implement Default twice, one implementation for the supported version
// and one for the unsupported version. Also, Default is not really a
// helpful concept here.

use std::{cell::RefCell, rc::Rc};

use crate::{
    engine::{DeviceSet, Engine, Name, Pool, PoolUuid},
    stratis::StratisResult,
};

#[cfg(feature = "dbus_enabled")]
use crate::{
    dbus_api::{DbusConnectionData, EventHandler},
    engine::get_engine_listener_list_mut,
};

pub struct MaybeDbusSupport {
    #[cfg(feature = "dbus_enabled")]
    handle: DbusConnectionData,
}

// If D-Bus compiled out, do very little.
#[cfg(not(feature = "dbus_enabled"))]
impl MaybeDbusSupport {
    pub fn setup(_engine: &Rc<RefCell<dyn Engine>>) -> StratisResult<MaybeDbusSupport> {
        Ok(MaybeDbusSupport {})
    }

    pub fn process(&mut self, _fds: &mut Vec<libc::pollfd>, _dbus_client_index_start: usize) {}

    pub fn register_pool(&mut self, _pool_name: &Name, _pool_uuid: PoolUuid, _pool: &mut dyn Pool) {
    }

    pub fn register_device_set(&mut self, _pool_uuid: PoolUuid, _device_set: &mut dyn DeviceSet) {}
}

#[cfg(feature = "dbus_enabled")]
impl MaybeDbusSupport {
    pub fn setup(engine: &Rc<RefCell<dyn Engine>>) -> StratisResult<MaybeDbusSupport> {
        DbusConnectionData::connect(Rc::clone(engine))
            .map(|mut handle| {
                let event_handler = Box::new(EventHandler::new(Rc::clone(&handle.connection)));
                get_engine_listener_list_mut().register_listener(event_handler);
                for (pool_name, pool_uuid, pool) in engine.borrow_mut().pools_mut() {
                    handle.register_pool(&pool_name, pool_uuid, pool)
                }
                info!("D-Bus API is available");
                MaybeDbusSupport { handle }
            })
            .map_err(|err| err.into())
    }

    /// Handle any client dbus requests.
    pub fn process(&mut self, fds: &mut Vec<libc::pollfd>, dbus_client_index_start: usize) {
        self.handle.handle(&fds[dbus_client_index_start..]);

        // Refresh list of dbus fds to poll for. This can change as
        // D-Bus clients come and go.
        fds.truncate(dbus_client_index_start);
        fds.extend(
            self.handle
                .connection
                .borrow()
                .watch_fds()
                .iter()
                .map(|w| w.to_pollfd()),
        );
    }

    pub fn register_pool(&mut self, pool_name: &Name, pool_uuid: PoolUuid, pool: &mut dyn Pool) {
        self.handle.register_pool(pool_name, pool_uuid, pool)
    }

    pub fn register_device_set(&mut self, pool_uuid: PoolUuid, device_set: &mut dyn DeviceSet) {
        self.handle.register_device_set(pool_uuid, device_set)
    }
}

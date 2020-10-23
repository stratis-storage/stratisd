// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Conditionally compiled support for a D-Bus interface.

// Allow new_without_default lint, because otherwise it would be necessary
// to implement Default twice, one implementation for the supported version
// and one for the unsupported version. Also, Default is not really a
// helpful concept here.

#![allow(dead_code)]

use std::{cell::RefCell, rc::Rc};

use crate::{
    dbus_api::{DbusConnectionData, EventHandler},
    engine::{get_engine_listener_list_mut, Engine, Name, Pool, PoolUuid},
    stratis::StratisResult,
};

pub struct IpcSupport {
    handle: DbusConnectionData,
}

impl IpcSupport {
    pub fn setup(engine: &Rc<RefCell<dyn Engine>>) -> StratisResult<IpcSupport> {
        DbusConnectionData::connect(Rc::clone(engine))
            .map(|mut handle| {
                let event_handler = Box::new(EventHandler::new(Rc::clone(&handle.connection)));
                get_engine_listener_list_mut().register_listener(event_handler);
                for (pool_name, pool_uuid, pool) in engine.borrow_mut().pools_mut() {
                    handle.register_pool(&pool_name, pool_uuid, pool)
                }
                info!("D-Bus API is available");
                IpcSupport { handle }
            })
            .map_err(|err| err.into())
    }

    pub fn register_pool(&mut self, pool_name: &Name, pool_uuid: PoolUuid, pool: &mut dyn Pool) {
        self.handle.register_pool(pool_name, pool_uuid, pool)
    }
}

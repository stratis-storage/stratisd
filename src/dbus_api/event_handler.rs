// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{cell::RefCell, rc::Rc};

use dbus::ffidisp::Connection;

use crate::{
    dbus_api::{consts, util::prop_changed_dispatch},
    engine::{EngineEvent, EngineListener, MaybeDbusPath},
};

#[derive(Debug)]
pub struct EventHandler {
    dbus_conn: Rc<RefCell<Connection>>,
}

impl EventHandler {
    pub fn new(dbus_conn: Rc<RefCell<Connection>>) -> EventHandler {
        EventHandler { dbus_conn }
    }
}

impl EngineListener for EventHandler {
    fn notify(&self, event: &EngineEvent) {
        match *event {
            EngineEvent::FilesystemRenamed {
                dbus_path,
                from,
                to,
            } => {
                if let MaybeDbusPath(Some(ref dbus_path)) = *dbus_path {
                    prop_changed_dispatch(
                        &self.dbus_conn.borrow(),
                        consts::FILESYSTEM_NAME_PROP,
                        to.to_string(),
                        dbus_path,
                        consts::FILESYSTEM_INTERFACE_NAME,
                    )
                    .unwrap_or_else(|()| {
                        warn!(
                            "FilesystemRenamed: {} from: {} to: {} failed to send dbus update.",
                            dbus_path, from, to,
                        );
                    });
                }
            }
            EngineEvent::PoolRenamed {
                dbus_path,
                from,
                to,
            } => {
                if let MaybeDbusPath(Some(ref dbus_path)) = *dbus_path {
                    prop_changed_dispatch(
                        &self.dbus_conn.borrow(),
                        consts::POOL_NAME_PROP,
                        to.to_string(),
                        dbus_path,
                        consts::POOL_INTERFACE_NAME,
                    )
                    .unwrap_or_else(|()| {
                        warn!(
                            "PoolRenamed: {} from: {} to: {} failed to send dbus update.",
                            dbus_path, from, to,
                        );
                    });
                }
            }
        }
    }
}

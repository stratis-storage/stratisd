// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{cell::RefCell, rc::Rc};

use dbus::Connection;

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
            EngineEvent::BlockdevStateChanged { dbus_path, state } => {
                if let MaybeDbusPath(Some(ref dbus_path)) = *dbus_path {
                    prop_changed_dispatch(
                        &self.dbus_conn.borrow(),
                        consts::BLOCKDEV_STATE_PROP,
                        state as u16,
                        &dbus_path,
                        consts::BLOCKDEV_INTERFACE_NAME,
                    )
                    .unwrap_or_else(|()| {
                        warn!(
                            "BlockdevStateChanged: {} state: {} failed to send dbus update.",
                            dbus_path, state as u16,
                        );
                    });
                }
            }
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
                        &dbus_path,
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
            EngineEvent::PoolExtendStateChanged { dbus_path, state } => {
                if let MaybeDbusPath(Some(ref dbus_path)) = *dbus_path {
                    prop_changed_dispatch(
                        &self.dbus_conn.borrow(),
                        consts::POOL_EXTEND_STATE_PROP,
                        state as u16,
                        &dbus_path,
                        consts::POOL_INTERFACE_NAME,
                    )
                    .unwrap_or_else(|()| {
                        warn!(
                            "PoolExtendStateChanged: {} state: {} failed to send dbus update.",
                            dbus_path, state as u16,
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
                        &dbus_path,
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
            EngineEvent::PoolSpaceStateChanged { dbus_path, state } => {
                if let MaybeDbusPath(Some(ref dbus_path)) = *dbus_path {
                    prop_changed_dispatch(
                        &self.dbus_conn.borrow(),
                        consts::POOL_SPACE_STATE_PROP,
                        state as u16,
                        &dbus_path,
                        consts::POOL_INTERFACE_NAME,
                    )
                    .unwrap_or_else(|()| {
                        warn!(
                            "PoolSpaceStateChanged: {} state: {} failed to send dbus update.",
                            dbus_path, state as u16,
                        );
                    });
                }
            }
            EngineEvent::PoolStateChanged { dbus_path, state } => {
                if let MaybeDbusPath(Some(ref dbus_path)) = *dbus_path {
                    prop_changed_dispatch(
                        &self.dbus_conn.borrow(),
                        consts::POOL_STATE_PROP,
                        state as u16,
                        &dbus_path,
                        consts::POOL_INTERFACE_NAME,
                    )
                    .unwrap_or_else(|()| {
                        warn!(
                            "PoolStateChanged: {} state: {} failed to send dbus update.",
                            dbus_path, state as u16,
                        );
                    });
                }
            }
        }
    }
}

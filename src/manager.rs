// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;
use dbus::{tree, Path};
use dbus::{Connection, BusType};
use dbus::tree::{Interface, Signal, MTFn, Access, MethodErr, EmitsChangedSignal};

use pool::TData;

pub struct Manager {
    pub name: String,
    pub path: Path<'static>,
}

impl Manager {

    pub fn new_manager(new_name: String) -> Manager {
        Manager {
            name: new_name,
            // TODO use a constant for object path
            path: format!("/org/storage/stratis/Manager"),
        }
    } 
}

pub fn create_manager_iface() -> (Interface<MTFn<TData>, TData>, Arc<Signal<TData>>) {

}

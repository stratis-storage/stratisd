// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate dbus;

use std::sync::Arc;
use std::cell::Cell;

use dbus::{tree, Path};

#[derive(Debug)]
pub struct Pool {
    pub name: String,
    pub path: Path<'static>,
    pub block_devs: BlockDevs,
    pub index: i32,
    pub online: Cell<bool>,
    pub checking: Cell<bool>,
}

#[derive(Copy, Clone, Default, Debug)]
pub struct TData;
impl tree::DataType for TData {
    type ObjectPath = Arc<Pool>;
    type Property = ();
    type Interface = ();
    type Method = ();
    type Signal = ();
}

impl Pool {
    pub fn new_pool(index: i32, new_name: String) -> Pool {
        Pool {
            name: new_name,
            // TODO use a constant for object path
            path: format!("/org/storage/stratis/{}", index).into(),
            index: index,
            online: Cell::new(index % 2 == 0),
            checking: Cell::new(false),
        }
    }
}

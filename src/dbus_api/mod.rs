// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[macro_use]
mod macros;

mod api;
mod blockdev;
mod connection;
mod consts;
mod filesystem;
mod pool;
mod types;
mod udev;
mod util;

use std::{
    collections::HashMap,
    sync::{Condvar, Mutex},
};

use dbus::Path;

use crate::engine::PoolUuid;

pub use self::{
    connection::{DbusConnectionHandler, DbusTreeHandler},
    udev::DbusUdevHandler,
    util::create_dbus_handlers,
};

type CreatePoolState = Mutex<HashMap<PoolUuid, Option<(Path<'static>, Vec<Path<'static>>)>>>;

lazy_static! {
    pub static ref POOL_CONDVAR: Condvar = Condvar::new();
    pub static ref POOL_SETUP_STATE: CreatePoolState = Mutex::new(HashMap::new());
}

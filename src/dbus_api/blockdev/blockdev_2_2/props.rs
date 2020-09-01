// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{
    arg::IterAppend,
    tree::{MTFn, MethodErr, PropInfo},
};

use crate::dbus_api::{
    blockdev::shared::{self, get_blockdev_property},
    types::TData,
};

/// Get the devnode for an object path.
pub fn get_blockdev_physical_path(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |_, p| Ok(shared::blockdev_physical_path_prop(p)))
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{
    arg::IterAppend,
    tree::{MTFn, MethodErr, PropInfo},
};

use crate::{
    dbus_api::{
        blockdev::shared::{self, blockdev_operation},
        types::TData,
    },
    engine::{BlockDev, BlockDevTier},
};

/// Get a blockdev property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to a
/// blockdev and obtains the property from the blockdev.
fn get_blockdev_property<F, R>(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
    getter: F,
) -> Result<(), MethodErr>
where
    F: Fn(BlockDevTier, &dyn BlockDev) -> Result<R, String>,
    R: dbus::arg::Append,
{
    i.append(
        blockdev_operation(p.tree, p.path.get_name(), getter)
            .map_err(|ref e| MethodErr::failed(e))?,
    );
    Ok(())
}

/// Get the devnode for an object path.
pub fn get_blockdev_physical_path(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |_, p| Ok(shared::blockdev_physical_path_prop(p)))
}

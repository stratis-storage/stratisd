// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{
    self,
    arg::IterAppend,
    tree::{MTFn, MethodErr, PropInfo},
};

use crate::{
    dbus_api::{blockdev::shared::blockdev_operation, types::TData},
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
pub fn get_blockdev_devnode(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |_, p| Ok(format!("{}", p.devnode().display())))
}

pub fn get_blockdev_hardware_info(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |_, p| {
        Ok(p.hardware_info()
            .map_or_else(|| (false, "".to_owned()), |val| (true, val.to_owned())))
    })
}

pub fn get_blockdev_user_info(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |_, p| {
        Ok(p.user_info()
            .map_or_else(|| (false, "".to_owned()), |val| (true, val.to_owned())))
    })
}

pub fn get_blockdev_initialization_time(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |_, p| Ok(p.initialization_time().timestamp() as u64))
}

pub fn get_blockdev_tier(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |t, _| Ok(t as u16))
}

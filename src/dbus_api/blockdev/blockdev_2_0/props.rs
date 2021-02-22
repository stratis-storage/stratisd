// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::arg::IterAppend;
use dbus_tree::{MTSync, MethodErr, PropInfo};

use crate::dbus_api::{
    blockdev::shared::{self, get_blockdev_property},
    types::TData,
};

/// Get the devnode for an object path.
pub fn get_blockdev_devnode(
    i: &mut IterAppend,
    p: &PropInfo<MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |_, p| Ok(shared::blockdev_devnode_prop(p)))
}

pub fn get_blockdev_hardware_info(
    i: &mut IterAppend,
    p: &PropInfo<MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |_, p| Ok(shared::blockdev_hardware_info_prop(p)))
}

pub fn get_blockdev_user_info(
    i: &mut IterAppend,
    p: &PropInfo<MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |_, p| Ok(shared::blockdev_user_info_prop(p)))
}

pub fn get_blockdev_initialization_time(
    i: &mut IterAppend,
    p: &PropInfo<MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |_, p| Ok(shared::blockdev_init_time_prop(p)))
}

pub fn get_blockdev_tier(
    i: &mut IterAppend,
    p: &PropInfo<MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    get_blockdev_property(i, p, |t, _| Ok(shared::blockdev_tier_prop(t)))
}

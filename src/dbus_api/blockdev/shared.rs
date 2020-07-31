// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{
    tree::{MTFn, Tree},
    Path,
};

use crate::{
    dbus_api::types::TData,
    engine::{BlockDev, BlockDevTier},
};

/// Perform an operation on a `BlockDev` object for a given
/// DBus implicit argument that is a block device
pub fn blockdev_operation<F, R>(
    tree: &Tree<MTFn<TData>, TData>,
    object_path: &Path<'static>,
    closure: F,
) -> Result<R, String>
where
    F: Fn(BlockDevTier, &dyn BlockDev) -> Result<R, String>,
    R: dbus::arg::Append,
{
    let dbus_context = tree.get_data();

    let blockdev_path = tree
        .get(object_path)
        .expect("tree must contain implicit argument");

    let blockdev_data = blockdev_path
        .get_data()
        .as_ref()
        .ok_or_else(|| format!("no data for object path {}", object_path))?;

    let pool_path = tree
        .get(&blockdev_data.parent)
        .ok_or_else(|| format!("no path for parent object path {}", &blockdev_data.parent))?;

    let pool_uuid = pool_path
        .get_data()
        .as_ref()
        .ok_or_else(|| format!("no data for object path {}", object_path))?
        .uuid;

    let engine = dbus_context.engine.borrow();
    let (_, pool) = engine
        .get_pool(pool_uuid)
        .ok_or_else(|| format!("no pool corresponding to uuid {}", &pool_uuid))?;
    let (tier, blockdev) = pool
        .get_blockdev(blockdev_data.uuid)
        .ok_or_else(|| format!("no blockdev with uuid {}", blockdev_data.uuid))?;
    closure(tier, blockdev)
}

/// Generate D-Bus representation of devnode property.
#[inline]
pub fn blockdev_devnode_prop(dev: &dyn BlockDev) -> String {
    dev.devnode().user_path().display().to_string()
}

/// Generate D-Bus representation of hardware info property.
#[inline]
pub fn blockdev_hardware_info_prop(dev: &dyn BlockDev) -> (bool, String) {
    dev.hardware_info()
        .map_or_else(|| (false, "".to_owned()), |val| (true, val.to_owned()))
}

/// Generate D-Bus representation of user info property.
#[inline]
pub fn blockdev_user_info_prop(dev: &dyn BlockDev) -> (bool, String) {
    dev.user_info()
        .map_or_else(|| (false, "".to_owned()), |val| (true, val.to_owned()))
}

/// Generate D-Bus representation of initialization time property.
#[inline]
pub fn blockdev_init_time_prop(dev: &dyn BlockDev) -> u64 {
    dev.initialization_time().timestamp() as u64
}

/// Generate D-Bus representation of tier property.
#[inline]
pub fn blockdev_tier_prop(tier: BlockDevTier) -> u16 {
    tier as u16
}

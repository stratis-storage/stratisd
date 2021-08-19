// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{arg::IterAppend, Path};
use dbus_tree::{MTSync, MethodErr, PropInfo, Tree};

use crate::{
    dbus_api::types::TData,
    engine::{BlockDev, BlockDevTier, Engine, Pool},
};

/// Perform an operation on a `BlockDev` object for a given
/// DBus implicit argument that is a block device
pub fn blockdev_operation<F, R, E>(
    tree: &Tree<MTSync<TData<E>>, TData<E>>,
    object_path: &Path<'static>,
    closure: F,
) -> Result<R, String>
where
    F: Fn(BlockDevTier, &<<E as Engine>::Pool as Pool>::BlockDev) -> Result<R, String>,
    R: dbus::arg::Append,
    E: Engine,
{
    let dbus_context = tree.get_data();

    let blockdev_path = tree
        .get(object_path)
        .expect("tree must contain implicit argument");

    let blockdev_data = blockdev_path
        .get_data()
        .as_ref()
        .ok_or_else(|| format!("no data for object path {}", object_path))?;
    let blockdev_uuid = typed_uuid_string_err!(blockdev_data.uuid; Dev);

    let pool_path = tree
        .get(&blockdev_data.parent)
        .ok_or_else(|| format!("no path for parent object path {}", &blockdev_data.parent))?;

    let pool_uuid = typed_uuid_string_err!(
        pool_path
            .get_data()
            .as_ref()
            .ok_or_else(|| format!("no data for object path {}", object_path))?
            .uuid;
        Pool
    );

    let mutex_lock = dbus_context.engine.blocking_lock();
    let (_, pool) = mutex_lock
        .get_pool(pool_uuid)
        .ok_or_else(|| format!("no pool corresponding to uuid {}", &pool_uuid))?;
    let (tier, blockdev) = pool
        .get_blockdev(blockdev_uuid)
        .ok_or_else(|| format!("no blockdev with uuid {}", blockdev_data.uuid))?;
    closure(tier, blockdev)
}

/// Get a blockdev property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to a
/// blockdev and obtains the property from the blockdev.
pub fn get_blockdev_property<F, R, E>(
    i: &mut IterAppend,
    p: &PropInfo<MTSync<TData<E>>, TData<E>>,
    getter: F,
) -> Result<(), MethodErr>
where
    F: Fn(BlockDevTier, &<<E as Engine>::Pool as Pool>::BlockDev) -> Result<R, String>,
    R: dbus::arg::Append,
    E: Engine,
{
    i.append(
        blockdev_operation(p.tree, p.path.get_name(), getter)
            .map_err(|ref e| MethodErr::failed(e))?,
    );
    Ok(())
}

/// Generate D-Bus representation of devnode property.
#[inline]
pub fn blockdev_devnode_prop<E>(dev: &<<E as Engine>::Pool as Pool>::BlockDev) -> String
where
    E: Engine,
{
    dev.metadata_path().display().to_string()
}

/// Generate D-Bus representation of hardware info property.
#[inline]
pub fn blockdev_hardware_info_prop<E>(
    dev: &<<E as Engine>::Pool as Pool>::BlockDev,
) -> (bool, String)
where
    E: Engine,
{
    dev.hardware_info()
        .map_or_else(|| (false, "".to_owned()), |val| (true, val.to_owned()))
}

/// Generate D-Bus representation of user info property.
#[inline]
pub fn blockdev_user_info_prop<E>(dev: &<<E as Engine>::Pool as Pool>::BlockDev) -> (bool, String)
where
    E: Engine,
{
    dev.user_info()
        .map_or_else(|| (false, "".to_owned()), |val| (true, val.to_owned()))
}

/// Generate D-Bus representation of initialization time property.
#[inline]
pub fn blockdev_init_time_prop<E>(dev: &<<E as Engine>::Pool as Pool>::BlockDev) -> u64
where
    E: Engine,
{
    dev.initialization_time().timestamp() as u64
}

/// Generate D-Bus representation of tier property.
#[inline]
pub fn blockdev_tier_prop(tier: BlockDevTier) -> u16 {
    tier as u16
}

// Generate a D-Bus representation of the physical path
#[inline]
pub fn blockdev_physical_path_prop<E>(dev: &<<E as Engine>::Pool as Pool>::BlockDev) -> String
where
    E: Engine,
{
    dev.devnode().display().to_string()
}

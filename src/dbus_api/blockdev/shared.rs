// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{arg::IterAppend, Path};
use dbus_tree::{MTSync, MethodErr, PropInfo, Tree};
use futures::executor::block_on;

use crate::{
    dbus_api::{blockdev::prop_conv, types::TData, util::option_to_tuple},
    engine::{
        BlockDev, BlockDevTier, DevUuid, Engine, LockKey, Name, Pool, PropChangeAction, ToDisplay,
    },
};

/// Perform a get operation on a `BlockDev` object for a given
/// DBus implicit argument that is a block device
pub fn blockdev_get_operation<F, R, E>(
    tree: &Tree<MTSync<TData<E>>, TData<E>>,
    object_path: &Path<'static>,
    closure: F,
) -> Result<R, String>
where
    F: Fn(BlockDevTier, &<E::Pool as Pool>::BlockDev) -> Result<R, String>,
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

    let pool = block_on(dbus_context.engine.get_pool(LockKey::Uuid(pool_uuid)))
        .ok_or_else(|| format!("no pool corresponding to uuid {}", &pool_uuid))?;
    let (tier, blockdev) = pool
        .get_blockdev(blockdev_uuid)
        .ok_or_else(|| format!("no blockdev with uuid {}", blockdev_data.uuid))?;
    closure(tier, blockdev)
}

/// Perform a set operation on a `BlockDev` object that requires a pool level API
/// operation for a given DBus implicit argument that is a block device
pub fn blockdev_pool_level_set_operation<F, R, E>(
    tree: &Tree<MTSync<TData<E>>, TData<E>>,
    object_path: &Path<'static>,
    closure: F,
) -> Result<R, String>
where
    F: Fn(&Name, &mut E::Pool, DevUuid) -> Result<R, String>,
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

    let mut pool = block_on(dbus_context.engine.get_mut_pool(LockKey::Uuid(pool_uuid)))
        .ok_or_else(|| format!("no pool corresponding to uuid {}", &pool_uuid))?;
    let (name, _, pool) = pool.as_mut_tuple();
    closure(&name, pool, blockdev_uuid)
}

/// Get a blockdev property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to a
/// blockdev and obtains the property from the blockdev.
pub fn get_blockdev_property<F, R, E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
    getter: F,
) -> Result<(), MethodErr>
where
    F: Fn(BlockDevTier, &<E::Pool as Pool>::BlockDev) -> Result<R, String>,
    R: dbus::arg::Append,
    E: Engine,
{
    i.append(
        blockdev_get_operation(p.tree, p.path.get_name(), getter)
            .map_err(|ref e| MethodErr::failed(e))?,
    );
    Ok(())
}

/// Set a blockdev property that needs to be set at the pool level.
pub fn set_pool_level_blockdev_property_to_display<F, R, E>(
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
    prop_name: &str,
    setter: F,
) -> Result<R, MethodErr>
where
    F: Fn(&Name, &mut E::Pool, DevUuid) -> Result<R, String>,
    E: Engine,
    R: ToDisplay,
{
    info!("Setting property {}", prop_name);
    let res = blockdev_pool_level_set_operation(p.tree, p.path.get_name(), setter)
        .map_err(|ref e| MethodErr::failed(e));
    let res_display = res.as_ref().map(|o| o.to_display());
    let _ = handle_action!(res_display);
    res
}

/// Generate D-Bus representation of devnode property.
#[inline]
pub fn blockdev_devnode_prop<E>(dev: &<E::Pool as Pool>::BlockDev) -> String
where
    E: Engine,
{
    dev.metadata_path().display().to_string()
}

/// Generate D-Bus representation of hardware info property.
#[inline]
pub fn blockdev_hardware_info_prop<E>(dev: &<E::Pool as Pool>::BlockDev) -> (bool, String)
where
    E: Engine,
{
    option_to_tuple(dev.hardware_info().map(|s| s.to_owned()), String::new())
}

/// Generate D-Bus representation of user info property.
#[inline]
pub fn blockdev_user_info_prop<E>(dev: &<E::Pool as Pool>::BlockDev) -> (bool, String)
where
    E: Engine,
{
    prop_conv::blockdev_user_info_to_prop(dev.user_info().map(|s| s.to_owned()))
}

/// Generate D-Bus representation of user info property.
#[inline]
pub fn set_blockdev_user_info_prop<'a, E>(
    pool: &mut E::Pool,
    pool_name: &Name,
    dev_uuid: DevUuid,
    user_info: Option<&'a str>,
) -> Result<PropChangeAction<Option<&'a str>>, String>
where
    E: Engine,
{
    if pool
        .get_blockdev(dev_uuid)
        .ok_or_else(|| format!("Blockdev with UUID {} not found", dev_uuid))?
        .1
        .user_info()
        == user_info
    {
        Ok(PropChangeAction::Identity)
    } else {
        pool.set_blockdev_user_info(pool_name, dev_uuid, user_info)
            .map(|_| PropChangeAction::NewValue(user_info))
            .map_err(|e| e.to_string())
    }
}

/// Generate D-Bus representation of initialization time property.
#[inline]
pub fn blockdev_init_time_prop<E>(dev: &<E::Pool as Pool>::BlockDev) -> u64
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

/// Generate a D-Bus representation of the physical path
#[inline]
pub fn blockdev_physical_path_prop<E>(dev: &<E::Pool as Pool>::BlockDev) -> String
where
    E: Engine,
{
    dev.devnode().display().to_string()
}

/// Generate D-Bus representation of devnode size.
#[inline]
pub fn blockdev_size_prop<E>(dev: &<E::Pool as Pool>::BlockDev) -> String
where
    E: Engine,
{
    (*dev.size().bytes()).to_string()
}

/// Generate D-Bus representation of new block device size.
#[inline]
pub fn blockdev_new_size_prop<E>(dev: &<E::Pool as Pool>::BlockDev) -> (bool, String)
where
    E: Engine,
{
    prop_conv::blockdev_new_size_to_prop(dev.new_size())
}

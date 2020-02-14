// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{
    arg::IterAppend,
    tree::{MTFn, MethodErr, MethodInfo, PropInfo, Tree},
};

use crate::{
    dbus_api::types::TData,
    engine::{Name, Pool, PoolUuid},
};

pub fn pool_operation<F, R>(
    tree: &Tree<MTFn<TData>, TData>,
    object_path: &dbus::Path<'static>,
    closure: F,
) -> Result<R, String>
where
    F: Fn((Name, PoolUuid, &dyn Pool)) -> Result<R, String>,
    R: dbus::arg::Append,
{
    let dbus_context = tree.get_data();

    let pool_path = tree
        .get(object_path)
        .expect("implicit argument must be in tree");

    let pool_uuid = pool_path
        .get_data()
        .as_ref()
        .ok_or_else(|| format!("no data for object path {}", object_path))?
        .uuid;

    let engine = dbus_context.engine.borrow();
    let (pool_name, pool) = engine
        .get_pool(pool_uuid)
        .ok_or_else(|| format!("no pool corresponding to uuid {}", &pool_uuid))?;

    closure((pool_name, pool_uuid, pool))
}

pub fn get_pool_has_cache(m: &MethodInfo<MTFn<TData>, TData>) -> Result<bool, String> {
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        Ok(pool.has_cache())
    })
}

pub fn get_pool_total_size(m: &MethodInfo<MTFn<TData>, TData>) -> Result<String, String> {
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        Ok(
            (u128::from(*pool.total_physical_size()) * devicemapper::SECTOR_SIZE as u128)
                .to_string(),
        )
    })
}

pub fn get_pool_total_used(m: &MethodInfo<MTFn<TData>, TData>) -> Result<String, String> {
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        pool.total_physical_used()
            .map_err(|e| e.to_string())
            .map(|size| (u128::from(*size) * devicemapper::SECTOR_SIZE as u128).to_string())
    })
}

/// Get a pool property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to a
/// Pool and obtains the property from the pool.
pub fn get_pool_property<F, R>(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
    getter: F,
) -> Result<(), MethodErr>
where
    F: Fn((Name, PoolUuid, &dyn Pool)) -> Result<R, String>,
    R: dbus::arg::Append,
{
    i.append(
        pool_operation(p.tree, p.path.get_name(), getter).map_err(|ref e| MethodErr::failed(e))?,
    );
    Ok(())
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;

use dbus::{
    arg::{Array, IterAppend},
    Message,
};
use dbus_tree::{MTSync, MethodErr, MethodInfo, MethodResult, PropInfo, Tree};
use futures::executor::block_on;

use crate::{
    dbus_api::{
        blockdev::create_dbus_blockdev,
        pool::prop_conv,
        types::{DbusErrorEnum, TData, OK_STRING},
        util::{engine_to_dbus_err_tuple, get_next_arg},
    },
    engine::{
        total_allocated, total_used, BlockDevTier, Diff, Engine, EngineAction, LockKey, Name, Pool,
        PoolUuid,
    },
};

pub enum BlockDevOp {
    InitCache,
    AddCache,
    AddData,
}

pub fn pool_operation<F, R, E>(
    tree: &Tree<MTSync<TData<E>>, TData<E>>,
    object_path: &dbus::Path<'static>,
    closure: F,
) -> Result<R, String>
where
    F: Fn((Name, PoolUuid, &E::Pool)) -> Result<R, String>,
    R: dbus::arg::Append,
    E: 'static + Engine,
{
    let dbus_context = tree.get_data();

    let pool_path = tree
        .get(object_path)
        .expect("implicit argument must be in tree");

    let pool_uuid = typed_uuid_string_err!(
        pool_path
            .get_data()
            .as_ref()
            .ok_or_else(|| format!("no data for object path {}", object_path))?
            .uuid;
        Pool
    );

    let guard = block_on(dbus_context.engine.get_pool(LockKey::Uuid(pool_uuid)))
        .ok_or_else(|| format!("no pool corresponding to uuid {}", &pool_uuid))?;
    let (pool_name, _, pool) = guard.as_tuple();

    closure((pool_name, pool_uuid, pool))
}

pub fn pool_set_operation<F, E>(
    tree: &Tree<MTSync<TData<E>>, TData<E>>,
    object_path: &dbus::Path<'static>,
    closure: F,
) -> Result<(), String>
where
    F: Fn((Name, PoolUuid, &mut E::Pool)) -> Result<(), String>,
    E: 'static + Engine,
{
    let dbus_context = tree.get_data();

    let pool_path = tree
        .get(object_path)
        .expect("implicit argument must be in tree");

    let pool_uuid = typed_uuid_string_err!(
        pool_path
            .get_data()
            .as_ref()
            .ok_or_else(|| format!("no data for object path {}", object_path))?
            .uuid;
        Pool
    );

    let mut guard = block_on(dbus_context.engine.get_mut_pool(LockKey::Uuid(pool_uuid)))
        .ok_or_else(|| format!("no pool corresponding to uuid {}", &pool_uuid))?;
    let (pool_name, _, pool) = guard.as_mut_tuple();

    closure((pool_name, pool_uuid, pool))
}

/// A method shared by all pool interfaces and by all blockdev-adding
/// operations, including cache initialization, which is considered a
/// blockdev-adding operation because when a cache is initialized, the
/// data tier must already contain some block devices. The op parameter
/// determines which method belonging to the engine's Pool interface must
/// be invoked.
pub fn add_blockdevs<E>(
    m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>,
    op: BlockDevOp,
) -> MethodResult
where
    E: 'static + Engine,
{
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let devs: Array<'_, &str, _> = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return: (bool, Vec<dbus::Path<'_>>) = (false, Vec::new());

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = typed_uuid!(
        get_data!(pool_path; default_return; return_message).uuid;
        Pool;
        default_return;
        return_message
    );

    let mut guard = get_mut_pool!(dbus_context.engine; pool_uuid; default_return; return_message);
    let (pool_name, _, pool) = guard.as_mut_tuple();

    let blockdevs = devs.map(Path::new).collect::<Vec<&Path>>();

    let result = match op {
        BlockDevOp::InitCache => {
            let res = handle_action!(
                pool.init_cache(pool_uuid, &*pool_name, &blockdevs),
                dbus_context,
                pool_path.get_name()
            );
            dbus_context.push_pool_cache_change(pool_path.get_name(), true);
            res
        }
        BlockDevOp::AddCache => {
            handle_action!(
                pool.add_blockdevs(pool_uuid, &*pool_name, &blockdevs, BlockDevTier::Cache,)
                    .map(|(act, _)| { act }),
                dbus_context,
                pool_path.get_name()
            )
        }
        BlockDevOp::AddData => {
            handle_action!(
                pool.add_blockdevs(pool_uuid, &*pool_name, &blockdevs, BlockDevTier::Data,)
                    .map(|(act, diff)| {
                        if act.is_changed() {
                            if let Some(d) = diff {
                                assert!(d.pool.metadata_size.is_changed());
                                dbus_context.push_pool_foreground_change(
                                    pool_path.get_name(),
                                    total_used(&d.thin_pool.used, &d.pool.metadata_size),
                                    total_allocated(
                                        &d.thin_pool.allocated_size,
                                        &d.pool.metadata_size,
                                    ),
                                    Diff::Changed(pool.total_physical_size().bytes()),
                                    d.pool.out_of_alloc_space,
                                )
                            }
                        }
                        act
                    }),
                dbus_context,
                pool_path.get_name()
            )
        }
    };
    let msg = match result.map(|bds| bds.changed()) {
        Ok(Some(uuids)) => {
            let return_value = uuids
                .iter()
                .map(|uuid| {
                    // FIXME: To avoid this expect, modify add_blockdevs
                    // so that it returns a mutable reference to each
                    // blockdev created.
                    create_dbus_blockdev(
                        dbus_context,
                        object_path.clone(),
                        *uuid,
                        match op {
                            BlockDevOp::AddData => BlockDevTier::Data,
                            _ => BlockDevTier::Cache,
                        },
                        pool.get_blockdev(*uuid)
                            .expect("just inserted by add_blockdevs")
                            .1,
                    )
                })
                .collect::<Vec<_>>();

            return_message.append3(
                (true, return_value),
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            )
        }
        Ok(None) => return_message.append3(
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };

    Ok(vec![msg])
}

/// Get a pool property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to a
/// Pool and obtains the property from the pool.
pub fn get_pool_property<F, R, E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
    getter: F,
) -> Result<(), MethodErr>
where
    F: Fn((Name, PoolUuid, &E::Pool)) -> Result<R, String>,
    R: dbus::arg::Append,
    E: 'static + Engine,
{
    #[allow(clippy::redundant_closure)]
    i.append(
        pool_operation(p.tree, p.path.get_name(), getter).map_err(|ref e| MethodErr::failed(e))?,
    );
    Ok(())
}

/// Set a pool property. The property is found by means of the setter method which
/// takes a mutable reference to a Pool and sets the property on the pool.
pub fn set_pool_property<F, E>(
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
    setter: F,
) -> Result<(), MethodErr>
where
    F: Fn((Name, PoolUuid, &mut E::Pool)) -> Result<(), String>,
    E: 'static + Engine,
{
    // Using clippy's suggested code causes a compilation error
    #[allow(clippy::redundant_closure)]
    pool_set_operation(p.tree, p.path.get_name(), setter).map_err(|ref e| MethodErr::failed(e))?;
    Ok(())
}

/// Generate D-Bus representation of name property.
#[inline]
pub fn pool_name_prop(name: &Name) -> String {
    name.to_owned()
}

/// Generate D-Bus representation of encrypted property.
#[inline]
pub fn pool_enc_prop<E>(pool: &E::Pool) -> bool
where
    E: 'static + Engine,
{
    pool.is_encrypted()
}

/// Generate D-Bus representation of pool state property.
pub fn pool_avail_actions_prop<E>(pool: &E::Pool) -> String
where
    E: 'static + Engine,
{
    prop_conv::avail_actions_to_prop(pool.avail_actions())
}

/// Generate D-Bus representation of a pool key description property.
pub fn pool_key_desc_prop<E>(pool: &E::Pool) -> (bool, (bool, String))
where
    E: 'static + Engine,
{
    prop_conv::key_desc_to_prop(pool.encryption_info())
}

/// Generate D-Bus representation of a pool Clevis info property.
pub fn pool_clevis_info_prop<E>(pool: &E::Pool) -> (bool, (bool, (String, String)))
where
    E: 'static + Engine,
{
    prop_conv::clevis_info_to_prop(pool.encryption_info())
}

/// Generate D-Bus representation of a boolean indicating whether the pool
/// has a cache.
#[inline]
pub fn pool_has_cache_prop<E>(pool: &E::Pool) -> bool
where
    E: 'static + Engine,
{
    pool.has_cache()
}

/// Generate D-Bus representation of the number of bytes of physical space
/// already allocated to this pool.
pub fn pool_allocated_size<E>(pool: &E::Pool) -> String
where
    E: 'static + Engine,
{
    prop_conv::pool_alloc_to_prop(pool.total_allocated_size().bytes())
}

/// Generate D-Bus representation of the number of bytes used by this pool.
pub fn pool_used_size<E>(pool: &E::Pool) -> (bool, String)
where
    E: 'static + Engine,
{
    prop_conv::pool_used_to_prop(pool.total_physical_used().map(|u| u.bytes()))
}

/// Generate a D-Bus representation of the total size of the pool in bytes.
#[inline]
pub fn pool_total_size<E>(pool: &E::Pool) -> String
where
    E: 'static + Engine,
{
    prop_conv::pool_size_to_prop(pool.total_physical_size().bytes())
}

/// Generate a D-Bus representation of the filesystem limit on the pool.
#[inline]
pub fn pool_fs_limit<E>(pool: &E::Pool) -> u64
where
    E: 'static + Engine,
{
    pool.fs_limit()
}

/// Set the filesystem limit on a pool.
#[inline]
pub fn set_pool_fs_limit<E>(
    name: &Name,
    pool_uuid: PoolUuid,
    pool: &mut E::Pool,
    new_limit: u64,
) -> Result<(), String>
where
    E: 'static + Engine,
{
    pool.set_fs_limit(name, pool_uuid, new_limit)
        .map_err(|e| e.to_string())
}

/// Generate a D-Bus representation of whether the pool has disabled overprovisioning
/// or not.
#[inline]
pub fn pool_overprov_enabled<E>(pool: &E::Pool) -> bool
where
    E: 'static + Engine,
{
    pool.overprov_enabled()
}

/// Set the overprovisioning mode on a pool.
#[inline]
pub fn pool_set_overprov_mode<E>(
    pool: &mut E::Pool,
    name: &Name,
    enabled: bool,
) -> Result<(), String>
where
    E: 'static + Engine,
{
    pool.set_overprov_mode(name, enabled)
        .map_err(|e| e.to_string())
}

/// Generate a D-Bus representation of whether the pool has remaining space to
/// allocate or not.
#[inline]
pub fn pool_no_alloc_space<E>(pool: &E::Pool) -> bool
where
    E: 'static + Engine,
{
    pool.out_of_alloc_space()
}

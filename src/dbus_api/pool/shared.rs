// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;

use dbus::{
    arg::{Array, IterAppend},
    Message,
};
use dbus_tree::{MTSync, MethodErr, MethodInfo, MethodResult, PropInfo, Tree};

use crate::{
    dbus_api::{
        blockdev::create_dbus_blockdev,
        types::{DbusErrorEnum, TData, OK_STRING},
        util::{engine_to_dbus_err_tuple, get_next_arg, option_to_tuple},
    },
    engine::{BlockDevTier, Engine, EngineAction, Name, Pool, PoolUuid},
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

    let mutex_lock = dbus_context.engine.blocking_lock();
    let (pool_name, pool) = mutex_lock
        .get_pool(pool_uuid)
        .ok_or_else(|| format!("no pool corresponding to uuid {}", &pool_uuid))?;

    closure((pool_name, pool_uuid, pool))
}

pub fn get_pool_encryption_key_desc<E>(
    m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(bool, String), String>
where
    E: 'static + Engine,
{
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        Ok(option_to_tuple(
            match pool.encryption_info() {
                Some(ei) => ei
                    .key_description()
                    .map_err(|e| e.to_string())?
                    .map(|kd| kd.as_application_str().to_string()),
                None => None,
            },
            String::new(),
        ))
    })
}

pub fn get_pool_has_cache<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> Result<bool, String>
where
    E: 'static + Engine,
{
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        Ok(pool.has_cache())
    })
}

pub fn get_pool_total_size<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> Result<String, String>
where
    E: 'static + Engine,
{
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        Ok((*pool.total_physical_size().bytes()).to_string())
    })
}

pub fn get_pool_total_used<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> Result<String, String>
where
    E: 'static + Engine,
{
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        pool.total_physical_used()
            .map_err(|e| e.to_string())
            .map(|size| (*size.bytes()).to_string())
    })
}

pub fn get_pool_clevis_info<E>(
    m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(bool, (String, String)), String>
where
    E: 'static + Engine,
{
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        Ok(option_to_tuple(
            match pool.encryption_info() {
                Some(ei) => ei
                    .clevis_info()
                    .map_err(|e| e.to_string())?
                    .map(|(pin, config)| (pin.to_owned(), config.to_string())),
                None => None,
            },
            (String::new(), String::new()),
        ))
    })
}

/// A method shared by all pool interfaces and by all blockdev-adding
/// operations, including cache initialization, which is considered a
/// blockdev-adding operation because when a cache is initialized, the
/// data tier must already contain some block devices. The op parameter
/// determines which method belonging to the engine's Pool interface must
/// be invoked.
pub fn add_blockdevs<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>, op: BlockDevOp) -> MethodResult
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

    let mut mutex_lock = dbus_context.engine.blocking_lock();
    let (pool_name, pool) = get_mut_pool!(mutex_lock; pool_uuid; default_return; return_message);

    let blockdevs = devs.map(|x| Path::new(x)).collect::<Vec<&Path>>();

    let result = match op {
        BlockDevOp::InitCache => handle_action!(
            pool.init_cache(pool_uuid, &*pool_name, &blockdevs),
            dbus_context,
            pool_path.get_name()
        ),
        BlockDevOp::AddCache => handle_action!(
            pool.add_blockdevs(pool_uuid, &*pool_name, &blockdevs, BlockDevTier::Cache),
            dbus_context,
            pool_path.get_name()
        ),
        BlockDevOp::AddData => handle_action!(
            pool.add_blockdevs(pool_uuid, &*pool_name, &blockdevs, BlockDevTier::Data),
            dbus_context,
            pool_path.get_name()
        ),
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
    i.append(
        pool_operation(p.tree, p.path.get_name(), getter).map_err(|ref e| MethodErr::failed(e))?,
    );
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
#[inline]
pub fn pool_avail_actions_prop<E>(pool: &E::Pool) -> String
where
    E: 'static + Engine,
{
    pool.avail_actions().to_string()
}

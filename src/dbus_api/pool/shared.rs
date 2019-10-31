// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;

use dbus::{
    arg::{Array, IterAppend},
    tree::{MTFn, MethodErr, MethodInfo, MethodResult, PropInfo, Tree},
    Message,
};

use crate::{
    dbus_api::{
        blockdev::create_dbus_blockdev,
        types::TData,
        util::{
            engine_to_dbus_err_tuple, get_next_arg, msg_code_ok, msg_string_ok, option_to_tuple,
        },
    },
    engine::{BlockDevTier, EngineAction, Name, Pool, PoolUuid},
};

pub enum BlockDevOp {
    InitCache,
    AddCache,
    AddData,
}

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

pub fn get_pool_encryption_key_desc(
    m: &MethodInfo<MTFn<TData>, TData>,
) -> Result<(bool, String), String> {
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        Ok(option_to_tuple(
            pool.key_desc().map(|key| key.to_string()),
            String::new(),
        ))
    })
}

pub fn get_pool_has_cache(m: &MethodInfo<MTFn<TData>, TData>) -> Result<bool, String> {
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        Ok(pool.has_cache())
    })
}

pub fn get_pool_total_size(m: &MethodInfo<MTFn<TData>, TData>) -> Result<String, String> {
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        Ok((*pool.total_physical_size().bytes()).to_string())
    })
}

pub fn get_pool_total_used(m: &MethodInfo<MTFn<TData>, TData>) -> Result<String, String> {
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        pool.total_physical_used()
            .map_err(|e| e.to_string())
            .map(|size| (*size.bytes()).to_string())
    })
}

/// A method shared by all pool interfaces and by all blockdev-adding
/// operations, including cache initialization, which is considered a
/// blockdev-adding operation because when a cache is initialized, the
/// data tier must already contain some block devices. The op parameter
/// determines which method belonging to the engine's Pool interface must
/// be invoked.
// FIXME: We have tried two approaches for dispatching to the proper methods
// in the Pool trait. The first had a lot of code duplication, which was
// a drawback. This approach has significantly less code duplication, but
// requires discovering the state of the cache, whether initialized or not,
// in order to figure out which method to dispatch to. We would like to find
// a third approach, which doesn't have the flaws of the two we've tried.
pub fn add_blockdevs(m: &MethodInfo<MTFn<TData>, TData>, op: BlockDevOp) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let devs: Array<&str, _> = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return: (bool, Vec<dbus::Path>) = (false, Vec::new());

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let (pool_name, pool) = get_mut_pool!(engine; pool_uuid; default_return; return_message);

    let blockdevs = devs.map(|x| Path::new(x)).collect::<Vec<&Path>>();

    let result = match op {
        BlockDevOp::InitCache => pool.init_cache(pool_uuid, &*pool_name, &blockdevs),
        BlockDevOp::AddCache => {
            pool.add_blockdevs(pool_uuid, &*pool_name, &blockdevs, BlockDevTier::Cache)
        }
        BlockDevOp::AddData => {
            pool.add_blockdevs(pool_uuid, &*pool_name, &blockdevs, BlockDevTier::Data)
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
                        pool.get_mut_blockdev(*uuid)
                            .expect("just inserted by add_blockdevs")
                            .1,
                    )
                })
                .collect::<Vec<_>>();

            return_message.append3((true, return_value), msg_code_ok(), msg_string_ok())
        }
        Ok(None) => return_message.append3(default_return, msg_code_ok(), msg_string_ok()),
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

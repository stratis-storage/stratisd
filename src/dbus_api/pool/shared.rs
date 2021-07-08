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
    engine::{BlockDevTier, EngineAction, MaybeInconsistent, Name, Pool, PoolUuid},
};

pub enum BlockDevOp {
    InitCache,
    AddCache,
    AddData,
}

pub fn pool_operation<F, R>(
    tree: &Tree<MTSync<TData>, TData>,
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

pub fn get_pool_encryption_key_desc(
    m: &MethodInfo<MTSync<TData>, TData>,
) -> Result<(bool, String), String> {
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        let key_description = match pool.encryption_info().key_description {
            Some(MaybeInconsistent::Yes) => {
                return Err(
                    "The key description metadata is inconsistent across devices in the pool"
                        .to_string(),
                )
            }
            Some(MaybeInconsistent::No(kd)) => Some(kd),
            None => None,
        };
        Ok(option_to_tuple(
            key_description
                .as_ref()
                .map(|kd| kd.as_application_str().to_string()),
            String::new(),
        ))
    })
}

pub fn get_pool_has_cache(m: &MethodInfo<MTSync<TData>, TData>) -> Result<bool, String> {
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        Ok(pool.has_cache())
    })
}

pub fn get_pool_total_size(m: &MethodInfo<MTSync<TData>, TData>) -> Result<String, String> {
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        Ok((*pool.total_physical_size().bytes()).to_string())
    })
}

pub fn get_pool_total_used(m: &MethodInfo<MTSync<TData>, TData>) -> Result<String, String> {
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        pool.total_physical_used()
            .map_err(|e| e.to_string())
            .map(|size| (*size.bytes()).to_string())
    })
}

pub fn get_pool_clevis_info(
    m: &MethodInfo<MTSync<TData>, TData>,
) -> Result<(bool, (String, String)), String> {
    pool_operation(m.tree, m.path.get_name(), |(_, _, pool)| {
        let clevis_info = match pool.encryption_info().clevis_info {
            Some(MaybeInconsistent::Yes) => {
                return Err("Clevis metadata is inconsistent across devices in the pool".to_string())
            }
            Some(MaybeInconsistent::No(ci)) => Some(ci),
            None => None,
        };
        Ok(option_to_tuple(
            clevis_info
                .as_ref()
                .map(|(pin, config)| (pin.to_owned(), config.to_string())),
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
pub fn add_blockdevs(m: &MethodInfo<MTSync<TData>, TData>, op: BlockDevOp) -> MethodResult {
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
pub fn get_pool_property<F, R>(
    i: &mut IterAppend,
    p: &PropInfo<MTSync<TData>, TData>,
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

/// Generate D-Bus representation of name property.
#[inline]
pub fn pool_name_prop(name: &Name) -> String {
    name.to_owned()
}

/// Generate D-Bus representation of encrypted property.
#[inline]
pub fn pool_enc_prop(pool: &dyn Pool) -> bool {
    pool.is_encrypted()
}

/// Generate D-Bus representation of pool state property.
#[inline]
pub fn pool_state_prop(pool: &dyn Pool) -> String {
    pool.pool_state().to_string()
}

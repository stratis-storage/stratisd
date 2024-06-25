// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, sync::Arc};

use dbus::arg::IterAppend;
use dbus_tree::{
    Factory, MTSync, Method, MethodErr, MethodInfo, MethodResult, ObjectPath, PropInfo, Tree,
};
use futures::executor::block_on;

use crate::{
    dbus_api::{
        api::prop_conv::{self, StoppedOrLockedPools},
        blockdev::get_blockdev_properties,
        filesystem::get_fs_properties,
        pool::get_pool_properties,
        types::{GetManagedObjects, InterfacesAddedThreadSafe, TData},
        util::thread_safe_to_dbus_sendable,
    },
    engine::{AllLockReadGuard, DevUuid, Engine, FilesystemUuid, Pool, PoolUuid, StratisUuid},
};

pub fn get_managed_objects_method(
    f: &Factory<MTSync<TData>, TData>,
) -> Method<MTSync<TData>, TData> {
    fn properties_to_get_managed_objects(
        path: dbus::Path<'static>,
        ia: InterfacesAddedThreadSafe,
    ) -> GetManagedObjects {
        let mut gmo = HashMap::new();
        gmo.insert(path, thread_safe_to_dbus_sendable(ia));
        gmo
    }

    fn pool_properties(
        path: &dbus::Path<'static>,
        table: &AllLockReadGuard<PoolUuid, dyn Pool>,
        pool_uuid: PoolUuid,
    ) -> Option<GetManagedObjects> {
        table.get_by_uuid(pool_uuid).map(|(n, p)| {
            properties_to_get_managed_objects(path.clone(), get_pool_properties(&n, pool_uuid, p))
        })
    }

    fn fs_properties(
        parent_path: &dbus::Path<'static>,
        path: &dbus::Path<'static>,
        table: &AllLockReadGuard<PoolUuid, dyn Pool>,
        pool_uuid: PoolUuid,
        fs_uuid: FilesystemUuid,
    ) -> Option<GetManagedObjects> {
        table.get_by_uuid(pool_uuid).and_then(|(p_n, p)| {
            p.get_filesystem(fs_uuid).map(|(ref fs_n, f)| {
                properties_to_get_managed_objects(
                    path.clone(),
                    get_fs_properties(parent_path.clone(), &p_n, fs_n, fs_uuid, f),
                )
            })
        })
    }

    fn blockdev_properties(
        parent_path: &dbus::Path<'static>,
        path: &dbus::Path<'static>,
        table: &AllLockReadGuard<PoolUuid, dyn Pool>,
        pool_uuid: PoolUuid,
        uuid: DevUuid,
    ) -> Option<GetManagedObjects> {
        table.get_by_uuid(pool_uuid).and_then(|(_, p)| {
            p.get_blockdev(uuid).map(|(bd_tier, bd)| {
                properties_to_get_managed_objects(
                    path.clone(),
                    get_blockdev_properties(parent_path.clone(), uuid, bd_tier, bd),
                )
            })
        })
    }

    fn parent_pool_uuid(op: Option<&ObjectPath<MTSync<TData>, TData>>) -> Option<PoolUuid> {
        op.and_then(|o| {
            o.get_data().as_ref().and_then(|data| match data.uuid {
                StratisUuid::Pool(p) => Some(p),
                _ => None,
            })
        })
    }

    fn get_managed_objects(m: &MethodInfo<'_, MTSync<TData>, TData>) -> MethodResult {
        let dbus_context = m.tree.get_data();

        let table = block_on(dbus_context.engine.pools());

        let properties: GetManagedObjects = m
            .tree
            .iter()
            .filter_map(|op| {
                op.get_data().as_ref().and_then(|data| match data.uuid {
                    StratisUuid::Pool(uuid) => pool_properties(op.get_name(), &table, uuid),
                    StratisUuid::Fs(uuid) => fs_properties(
                        &data.parent,
                        op.get_name(),
                        &table,
                        parent_pool_uuid(m.tree.get(&data.parent).map(|p| &**p))
                            .expect("Parent must be present and be pool"),
                        uuid,
                    ),
                    StratisUuid::Dev(uuid) => blockdev_properties(
                        &data.parent,
                        op.get_name(),
                        &table,
                        parent_pool_uuid(m.tree.get(&data.parent).map(|p| &**p))
                            .expect("Parent must be present and be pool"),
                        uuid,
                    ),
                })
            })
            .fold(HashMap::new(), |mut props, prop| {
                props.extend(prop);
                props
            });

        Ok(vec![m.msg.method_return().append1(properties)])
    }

    f.method("GetManagedObjects", (), get_managed_objects)
        .out_arg(("objpath_interfaces_and_properties", "a{oa{sa{sv}}}"))
}

/// Get a Manager property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to an
/// engine and obtains the property from the engine.
pub fn get_manager_property<F, R>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
    getter: F,
) -> Result<(), MethodErr>
where
    F: Fn(Arc<dyn Engine>) -> Result<R, String>,
    R: dbus::arg::Append,
{
    i.append(manager_operation(p.tree, getter).map_err(|ref e| MethodErr::failed(e))?);
    Ok(())
}

/// Perform an operation on an Engine object for a given D-Bus implicit argument
/// that is a Manager
pub fn manager_operation<F, R>(tree: &Tree<MTSync<TData>, TData>, closure: F) -> Result<R, String>
where
    F: Fn(Arc<dyn Engine>) -> Result<R, String>,
    R: dbus::arg::Append,
{
    let dbus_context = tree.get_data();
    closure(Arc::clone(&dbus_context.engine))
}

/// Generate D-Bus representation of locked pools
#[inline]
pub fn locked_pools_prop(e: Arc<dyn Engine>) -> StoppedOrLockedPools {
    prop_conv::locked_pools_to_prop(&block_on(e.locked_pools()))
}

/// Generate D-Bus representation of stopped pools
#[inline]
pub fn stopped_pools_prop(e: Arc<dyn Engine>, metadata: bool) -> StoppedOrLockedPools {
    prop_conv::stopped_pools_to_prop(&block_on(e.stopped_pools()), metadata)
}

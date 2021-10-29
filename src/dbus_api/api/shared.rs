// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use dbus_tree::{Factory, MTSync, Method, MethodInfo, MethodResult, ObjectPath};

use crate::{
    dbus_api::{
        blockdev::get_blockdev_properties,
        filesystem::get_fs_properties,
        pool::get_pool_properties,
        types::{GetManagedObjects, InterfacesAddedThreadSafe, TData},
        util::thread_safe_to_dbus_sendable,
    },
    engine::{DevUuid, Engine, FilesystemUuid, Pool, PoolUuid, StratisUuid},
};

pub fn get_managed_objects_method<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Method<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    fn properties_to_get_managed_objects(
        path: dbus::Path<'static>,
        ia: InterfacesAddedThreadSafe,
    ) -> GetManagedObjects {
        let mut gmo = HashMap::new();
        gmo.insert(path, thread_safe_to_dbus_sendable(ia));
        gmo
    }

    fn pool_properties<E>(
        path: &dbus::Path<'static>,
        engine: &E,
        pool_uuid: PoolUuid,
    ) -> Option<GetManagedObjects>
    where
        E: 'static + Engine,
    {
        engine.get_pool(pool_uuid).map(|(ref n, p)| {
            properties_to_get_managed_objects(
                path.clone(),
                get_pool_properties::<E>(n, pool_uuid, p),
            )
        })
    }

    fn fs_properties<E>(
        parent_path: &dbus::Path<'static>,
        path: &dbus::Path<'static>,
        engine: &E,
        pool_uuid: PoolUuid,
        fs_uuid: FilesystemUuid,
    ) -> Option<GetManagedObjects>
    where
        E: 'static + Engine,
    {
        engine.get_pool(pool_uuid).and_then(|(ref p_n, p)| {
            p.get_filesystem(fs_uuid).map(|(ref fs_n, f)| {
                properties_to_get_managed_objects(
                    path.clone(),
                    get_fs_properties::<E>(parent_path.clone(), p_n, fs_n, fs_uuid, f),
                )
            })
        })
    }

    fn blockdev_properties<E>(
        parent_path: &dbus::Path<'static>,
        path: &dbus::Path<'static>,
        engine: &E,
        pool_uuid: PoolUuid,
        uuid: DevUuid,
    ) -> Option<GetManagedObjects>
    where
        E: 'static + Engine,
    {
        engine.get_pool(pool_uuid).and_then(|(_, p)| {
            p.get_blockdev(uuid).map(|(bd_tier, bd)| {
                properties_to_get_managed_objects(
                    path.clone(),
                    get_blockdev_properties::<E>(parent_path.clone(), uuid, bd_tier, bd),
                )
            })
        })
    }

    fn parent_pool_uuid<E>(op: Option<&ObjectPath<MTSync<TData<E>>, TData<E>>>) -> Option<PoolUuid>
    where
        E: 'static + Engine,
    {
        op.and_then(|o| {
            o.get_data().as_ref().and_then(|data| match data.uuid {
                StratisUuid::Pool(p) => Some(p),
                _ => None,
            })
        })
    }

    fn get_managed_objects<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> MethodResult
    where
        E: 'static + Engine,
    {
        let dbus_context = m.tree.get_data();
        let engine = dbus_context.engine.blocking_lock();

        let properties: GetManagedObjects = m
            .tree
            .iter()
            .filter_map(|op| {
                op.get_data().as_ref().and_then(|data| match data.uuid {
                    StratisUuid::Pool(uuid) => pool_properties(op.get_name(), &*engine, uuid),
                    StratisUuid::Fs(uuid) => fs_properties(
                        &data.parent,
                        op.get_name(),
                        &*engine,
                        parent_pool_uuid(m.tree.get(&data.parent).map(|p| &**p))
                            .expect("Parent must be present and be pool"),
                        uuid,
                    ),
                    StratisUuid::Dev(uuid) => blockdev_properties(
                        &data.parent,
                        op.get_name(),
                        &*engine,
                        parent_pool_uuid(m.tree.get(&data.parent).map(|p| &**p))
                            .expect("Parent must be present and be pool"),
                        uuid,
                    ),
                })
            })
            .fold(HashMap::new(), |mut props, prop| {
                props.extend(prop.into_iter());
                props
            });

        Ok(vec![m.msg.method_return().append1(properties)])
    }

    f.method("GetManagedObjects", (), get_managed_objects)
}

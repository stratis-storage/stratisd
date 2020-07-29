// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, iter::FromIterator};

use dbus::{
    arg::{RefArg, Variant},
    tree::Factory,
};

use crate::{
    dbus_api::{
        consts,
        types::{DbusContext, OPContext, ObjectPathType},
        util::make_object_path,
    },
    engine::{MaybeDbusPath, Name, Pool, PoolUuid},
};

mod fetch_properties_2_0;
mod fetch_properties_2_1;
mod pool_2_0;
mod pool_2_1;
mod shared;

pub fn create_dbus_pool<'a>(
    dbus_context: &DbusContext,
    parent: dbus::Path<'static>,
    name: &Name,
    uuid: PoolUuid,
    pool: &mut dyn Pool,
) -> dbus::Path<'a> {
    let f = Factory::new_fn();

    let object_name = make_object_path(dbus_context);

    let object_path = f
        .object_path(
            object_name,
            Some(OPContext::new(parent, uuid, ObjectPathType::Pool)),
        )
        .introspectable()
        .add(
            f.interface(consts::POOL_INTERFACE_NAME, ())
                .add_m(pool_2_0::create_filesystems_method(&f))
                .add_m(pool_2_0::destroy_filesystems_method(&f))
                .add_m(pool_2_0::snapshot_filesystem_method(&f))
                .add_m(pool_2_0::add_blockdevs_method(&f))
                .add_m(pool_2_0::add_cachedevs_method(&f))
                .add_m(pool_2_0::rename_method(&f))
                .add_p(pool_2_0::name_property(&f))
                .add_p(pool_2_0::uuid_property(&f)),
        )
        .add(
            f.interface(consts::POOL_INTERFACE_NAME_2_1, ())
                .add_m(pool_2_0::create_filesystems_method(&f))
                .add_m(pool_2_0::destroy_filesystems_method(&f))
                .add_m(pool_2_0::snapshot_filesystem_method(&f))
                .add_m(pool_2_0::add_blockdevs_method(&f))
                .add_m(pool_2_1::init_cache_method(&f))
                .add_m(pool_2_1::add_cachedevs_method(&f))
                .add_m(pool_2_0::rename_method(&f))
                .add_p(pool_2_0::name_property(&f))
                .add_p(pool_2_0::uuid_property(&f))
                .add_p(pool_2_1::encrypted_property(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME, ())
                .add_m(fetch_properties_2_0::get_all_properties_method(&f))
                .add_m(fetch_properties_2_0::get_properties_method(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME_2_1, ())
                .add_m(fetch_properties_2_1::get_all_properties_method(&f))
                .add_m(fetch_properties_2_1::get_properties_method(&f)),
        );

    let path = object_path.get_name().to_owned();
    let interfaces = get_inital_properties(name, uuid, pool);
    dbus_context
        .actions
        .borrow_mut()
        .push_add(object_path, interfaces);
    pool.set_dbus_path(MaybeDbusPath(Some(path.clone())));
    path
}

/// Get the initial state of all properties associated with a pool object.
pub fn get_inital_properties(
    pool_name: &Name,
    pool_uuid: PoolUuid,
    pool: &dyn Pool,
) -> HashMap<String, HashMap<String, Variant<Box<dyn RefArg>>>> {
    let pool_iface_map = HashMap::from_iter(
        vec![
            (
                consts::POOL_NAME_PROP,
                Variant(Box::new(pool_name.to_string()) as Box<dyn RefArg>),
            ),
            (
                consts::POOL_UUID_PROP,
                Variant(Box::new(pool_uuid.to_simple_ref().to_string()) as Box<dyn RefArg>),
            ),
        ]
        .into_iter()
        .map(|(key, val)| (key.to_string(), val)),
    );

    let pool_2_1_iface_map = HashMap::from_iter(
        vec![
            (
                consts::POOL_NAME_PROP,
                Variant(Box::new(pool_name.to_string()) as Box<dyn RefArg>),
            ),
            (
                consts::POOL_UUID_PROP,
                Variant(Box::new(pool_uuid.to_simple_ref().to_string()) as Box<dyn RefArg>),
            ),
            (
                consts::POOL_UUID_PROP,
                Variant(Box::new(pool.is_encrypted()) as Box<dyn RefArg>),
            ),
        ]
        .into_iter()
        .map(|(key, val)| (key.to_string(), val)),
    );

    HashMap::from_iter(
        vec![
            (consts::POOL_INTERFACE_NAME, pool_iface_map),
            (consts::POOL_INTERFACE_NAME_2_1, pool_2_1_iface_map),
            (consts::PROPERTY_FETCH_INTERFACE_NAME, HashMap::new()),
            (consts::PROPERTY_FETCH_INTERFACE_NAME_2_1, HashMap::new()),
        ]
        .into_iter()
        .map(|(key, val)| (key.to_string(), val)),
    )
}

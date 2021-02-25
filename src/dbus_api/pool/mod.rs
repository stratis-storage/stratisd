// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::arg::Variant;
use dbus_tree::Factory;

use crate::{
    dbus_api::{
        consts,
        types::{DbusContext, InterfacesAdded, OPContext, ObjectPathType},
        util::make_object_path,
    },
    engine::{Name, Pool, PoolUuid, StratisUuid},
};

mod fetch_properties_2_0;
mod fetch_properties_2_1;
mod fetch_properties_2_3;
mod pool_2_0;
mod pool_2_1;
mod pool_2_3;
mod shared;

pub fn create_dbus_pool<'a>(
    dbus_context: &DbusContext,
    parent: dbus::Path<'static>,
    name: &Name,
    uuid: PoolUuid,
    pool: &dyn Pool,
) -> dbus::Path<'a> {
    let f = Factory::new_sync();

    let object_name = make_object_path(dbus_context);

    let object_path = f
        .object_path(
            object_name,
            Some(OPContext::new(
                parent,
                StratisUuid::Pool(uuid),
                ObjectPathType::Pool,
            )),
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
            f.interface(consts::POOL_INTERFACE_NAME_2_3, ())
                .add_m(pool_2_0::create_filesystems_method(&f))
                .add_m(pool_2_0::destroy_filesystems_method(&f))
                .add_m(pool_2_0::snapshot_filesystem_method(&f))
                .add_m(pool_2_0::add_blockdevs_method(&f))
                .add_m(pool_2_3::bind_clevis_method(&f))
                .add_m(pool_2_3::unbind_clevis_method(&f))
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
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME_2_2, ())
                .add_m(fetch_properties_2_1::get_all_properties_method(&f))
                .add_m(fetch_properties_2_1::get_properties_method(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME_2_3, ())
                .add_m(fetch_properties_2_3::get_all_properties_method(&f))
                .add_m(fetch_properties_2_3::get_properties_method(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME_2_4, ())
                .add_m(fetch_properties_2_3::get_all_properties_method(&f))
                .add_m(fetch_properties_2_3::get_properties_method(&f)),
        );

    let path = object_path.get_name().to_owned();
    let interfaces = get_initial_properties(name, uuid, pool);
    dbus_context.push_add(object_path, interfaces);
    path
}

/// Get the initial state of all properties associated with a pool object.
pub fn get_initial_properties(
    pool_name: &Name,
    pool_uuid: PoolUuid,
    pool: &dyn Pool,
) -> InterfacesAdded {
    initial_properties! {
        consts::POOL_INTERFACE_NAME => {
            consts::POOL_NAME_PROP => shared::pool_name_prop(pool_name),
            consts::POOL_UUID_PROP => uuid_to_string!(pool_uuid)
        },
        consts::POOL_INTERFACE_NAME_2_1 => {
            consts::POOL_NAME_PROP => shared::pool_name_prop(pool_name),
            consts::POOL_UUID_PROP => uuid_to_string!(pool_uuid),
            consts::POOL_ENCRYPTED_PROP => shared::pool_enc_prop(pool)
        }
    }
}

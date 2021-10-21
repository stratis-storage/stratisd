// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::Factory;

use crate::{
    dbus_api::{
        consts,
        types::{DbusContext, InterfacesAddedThreadSafe, OPContext},
        util::make_object_path,
    },
    engine::{Engine, Name, PoolUuid, StratisUuid},
};

mod fetch_properties_3_0;
mod pool_3_0;
mod shared;

pub fn create_dbus_pool<'a, E>(
    dbus_context: &DbusContext<E>,
    parent: dbus::Path<'static>,
    name: &Name,
    uuid: PoolUuid,
    pool: &E::Pool,
) -> dbus::Path<'a>
where
    E: 'static + Engine,
{
    let f = Factory::new_sync();

    let object_name = make_object_path(dbus_context);

    let object_path = f
        .object_path(
            object_name,
            Some(OPContext::new(parent, StratisUuid::Pool(uuid))),
        )
        .introspectable()
        .add(
            f.interface(consts::POOL_INTERFACE_NAME_3_0, ())
                .add_m(pool_3_0::create_filesystems_method(&f))
                .add_m(pool_3_0::destroy_filesystems_method(&f))
                .add_m(pool_3_0::snapshot_filesystem_method(&f))
                .add_m(pool_3_0::add_blockdevs_method(&f))
                .add_m(pool_3_0::bind_clevis_method(&f))
                .add_m(pool_3_0::unbind_clevis_method(&f))
                .add_m(pool_3_0::init_cache_method(&f))
                .add_m(pool_3_0::add_cachedevs_method(&f))
                .add_m(pool_3_0::bind_keyring_method(&f))
                .add_m(pool_3_0::unbind_keyring_method(&f))
                .add_m(pool_3_0::rebind_keyring_method(&f))
                .add_m(pool_3_0::rebind_clevis_method(&f))
                .add_m(pool_3_0::rename_method(&f))
                .add_p(pool_3_0::name_property(&f))
                .add_p(pool_3_0::uuid_property(&f))
                .add_p(pool_3_0::encrypted_property(&f))
                .add_p(pool_3_0::avail_actions_property(&f))
                .add_p(pool_3_0::key_desc_property(&f))
                .add_p(pool_3_0::has_cache_property(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME_3_0, ())
                .add_m(fetch_properties_3_0::get_all_properties_method(&f))
                .add_m(fetch_properties_3_0::get_properties_method(&f)),
        );

    let path = object_path.get_name().to_owned();
    let interfaces = get_pool_properties::<E>(name, uuid, pool);
    dbus_context.push_add(object_path, interfaces);
    path
}

/// Get the initial state of all properties associated with a pool object.
pub fn get_pool_properties<E>(
    pool_name: &Name,
    pool_uuid: PoolUuid,
    pool: &E::Pool,
) -> InterfacesAddedThreadSafe
where
    E: 'static + Engine,
{
    initial_properties! {
        consts::POOL_INTERFACE_NAME_3_0 => {
            consts::POOL_NAME_PROP => shared::pool_name_prop(pool_name),
            consts::POOL_UUID_PROP => uuid_to_string!(pool_uuid),
            consts::POOL_ENCRYPTED_PROP => shared::pool_enc_prop::<E>(pool),
            consts::POOL_AVAIL_ACTIONS_PROP => shared::pool_avail_actions_prop::<E>(pool),
            consts::POOL_KEY_DESC_PROP => shared::pool_key_desc_prop::<E>(pool),
            consts::POOL_HAS_CACHE_PROP => shared::pool_has_cache_prop::<E>(pool)
        }
    }
}

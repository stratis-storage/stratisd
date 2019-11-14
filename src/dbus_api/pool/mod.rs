// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{self, tree::Factory};

use crate::{
    dbus_api::{
        consts,
        pool::{
            fetch_properties_2_0::api::{get_all_properties_method, get_properties_method},
            pool_2_0::api::{
                add_blockdevs_method, add_cachedevs_method, create_filesystems_method,
                destroy_filesystems_method, name_property, rename_method,
                snapshot_filesystem_method, uuid_property,
            },
            pool_2_1::api::{add_cachedevs_2_1_method, init_cache_2_1_method},
        },
        types::{DbusContext, OPContext},
        util::make_object_path,
    },
    engine::{MaybeDbusPath, Pool, PoolUuid},
};

pub mod fetch_properties_2_0;
pub mod pool_2_0;
pub mod pool_2_1;
pub mod shared;

pub fn create_dbus_pool<'a>(
    dbus_context: &DbusContext,
    parent: dbus::Path<'static>,
    uuid: PoolUuid,
    pool: &mut dyn Pool,
) -> dbus::Path<'a> {
    let f = Factory::new_fn();

    let object_name = make_object_path(dbus_context);

    let object_path = f
        .object_path(object_name, Some(OPContext::new(parent, uuid)))
        .introspectable()
        .add(
            f.interface(consts::POOL_INTERFACE_NAME, ())
                .add_m(create_filesystems_method(&f))
                .add_m(destroy_filesystems_method(&f))
                .add_m(snapshot_filesystem_method(&f))
                .add_m(add_blockdevs_method(&f))
                .add_m(add_cachedevs_method(&f))
                .add_m(rename_method(&f))
                .add_p(name_property(&f))
                .add_p(uuid_property(&f)),
        )
        .add(
            f.interface(consts::POOL_INTERFACE_NAME_2_1, ())
                .add_m(create_filesystems_method(&f))
                .add_m(destroy_filesystems_method(&f))
                .add_m(snapshot_filesystem_method(&f))
                .add_m(add_blockdevs_method(&f))
                .add_m(init_cache_2_1_method(&f))
                .add_m(add_cachedevs_2_1_method(&f))
                .add_m(rename_method(&f))
                .add_p(name_property(&f))
                .add_p(uuid_property(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME, ())
                .add_m(get_all_properties_method(&f))
                .add_m(get_properties_method(&f)),
        );

    let path = object_path.get_name().to_owned();
    dbus_context.actions.borrow_mut().push_add(object_path);
    pool.set_dbus_path(MaybeDbusPath(Some(path.clone())));
    path
}

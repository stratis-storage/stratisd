// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Factory, MTSync, Tree};

use crate::dbus_api::{
    consts,
    types::{DbusContext, TData},
};

mod fetch_properties_2_1;
mod fetch_properties_2_2;
mod fetch_properties_2_4;
mod manager_2_0;
mod manager_2_1;
mod manager_2_2;
mod manager_2_3;
mod manager_2_4;
mod report_2_1;
mod shared;

pub fn get_base_tree<'a>(
    dbus_context: DbusContext,
) -> (Tree<MTSync<TData>, TData>, dbus::Path<'a>) {
    let f = Factory::new_sync();

    let base_tree = f.tree(dbus_context);

    let obj_path = f.object_path("/", None).introspectable();
    let base_tree = base_tree.add(obj_path);

    let obj_path = f.object_path("/org", None).introspectable();
    let base_tree = base_tree.add(obj_path);

    let obj_path = f.object_path("/org/storage", None).introspectable();
    let base_tree = base_tree.add(obj_path);

    let obj_path = f
        .object_path(consts::STRATIS_BASE_PATH, None)
        .introspectable()
        .object_manager()
        .add(
            f.interface(consts::MANAGER_INTERFACE_NAME, ())
                .add_m(manager_2_0::create_pool_method(&f))
                .add_m(manager_2_0::destroy_pool_method(&f))
                .add_m(manager_2_0::configure_simulator_method(&f))
                .add_p(manager_2_0::version_property(&f)),
        )
        .add(
            f.interface(consts::MANAGER_INTERFACE_NAME_2_1, ())
                .add_m(manager_2_1::create_pool_method(&f))
                .add_m(manager_2_1::set_key_method(&f))
                .add_m(manager_2_1::unset_key_method(&f))
                .add_m(manager_2_1::unlock_pool_method(&f))
                .add_m(manager_2_0::destroy_pool_method(&f))
                .add_m(manager_2_0::configure_simulator_method(&f))
                .add_p(manager_2_0::version_property(&f)),
        )
        .add(
            f.interface(consts::MANAGER_INTERFACE_NAME_2_2, ())
                .add_m(manager_2_1::create_pool_method(&f))
                .add_m(manager_2_2::set_key_method(&f))
                .add_m(manager_2_1::unset_key_method(&f))
                .add_m(manager_2_1::unlock_pool_method(&f))
                .add_m(manager_2_0::destroy_pool_method(&f))
                .add_m(manager_2_0::configure_simulator_method(&f))
                .add_p(manager_2_0::version_property(&f)),
        )
        .add(
            f.interface(consts::MANAGER_INTERFACE_NAME_2_3, ())
                .add_m(manager_2_1::create_pool_method(&f))
                .add_m(manager_2_2::set_key_method(&f))
                .add_m(manager_2_1::unset_key_method(&f))
                .add_m(manager_2_3::unlock_pool_method(&f))
                .add_m(manager_2_0::destroy_pool_method(&f))
                .add_m(manager_2_0::configure_simulator_method(&f))
                .add_p(manager_2_0::version_property(&f)),
        )
        .add(
            f.interface(consts::MANAGER_INTERFACE_NAME_2_4, ())
                .add_m(manager_2_4::create_pool_method(&f))
                .add_m(manager_2_2::set_key_method(&f))
                .add_m(manager_2_1::unset_key_method(&f))
                .add_m(manager_2_3::unlock_pool_method(&f))
                .add_m(manager_2_0::destroy_pool_method(&f))
                .add_m(manager_2_0::configure_simulator_method(&f))
                .add_m(manager_2_4::engine_state_report_method(&f))
                .add_p(manager_2_0::version_property(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME_2_1, ())
                .add_m(fetch_properties_2_1::get_all_properties_method(&f))
                .add_m(fetch_properties_2_1::get_properties_method(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME_2_2, ())
                .add_m(fetch_properties_2_2::get_all_properties_method(&f))
                .add_m(fetch_properties_2_2::get_properties_method(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME_2_3, ())
                .add_m(fetch_properties_2_2::get_all_properties_method(&f))
                .add_m(fetch_properties_2_2::get_properties_method(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME_2_4, ())
                .add_m(fetch_properties_2_4::get_all_properties_method(&f))
                .add_m(fetch_properties_2_4::get_properties_method(&f)),
        )
        .add(
            f.interface(consts::REPORT_INTERFACE_NAME_2_1, ())
                .add_m(report_2_1::get_report_method(&f)),
        );

    let path = obj_path.get_name().to_owned();
    (base_tree.add(obj_path), path)
}

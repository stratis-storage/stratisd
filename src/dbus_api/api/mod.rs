// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{
    self,
    tree::{Factory, MTFn, Tree},
};

use crate::dbus_api::{
    api::manager_2_0::api::{
        configure_simulator_method, create_pool_2_0_method, destroy_pool_method, version_property,
    },
    api::manager_2_1::api::create_pool_2_1_method,
    consts,
    types::{DbusContext, TData},
};

pub mod manager_2_0;
pub mod manager_2_1;
pub mod shared;

pub fn get_base_tree<'a>(dbus_context: DbusContext) -> (Tree<MTFn<TData>, TData>, dbus::Path<'a>) {
    let f = Factory::new_fn();

    let base_tree = f.tree(dbus_context);

    let obj_path = f
        .object_path(consts::STRATIS_BASE_PATH, None)
        .introspectable()
        .object_manager()
        .add(
            f.interface(consts::MANAGER_INTERFACE_NAME, ())
                .add_m(create_pool_2_0_method(&f))
                .add_m(destroy_pool_method(&f))
                .add_m(configure_simulator_method(&f))
                .add_p(version_property(&f)),
        )
        .add(
            f.interface(consts::MANAGER_INTERFACE_NAME_2_1, ())
                .add_m(create_pool_2_1_method(&f))
                .add_m(destroy_pool_method(&f))
                .add_m(configure_simulator_method(&f))
                .add_p(version_property(&f)),
        );

    let path = obj_path.get_name().to_owned();
    (base_tree.add(obj_path), path)
}

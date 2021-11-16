// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Factory, MTSync, Tree};

use crate::{
    dbus_api::{
        consts,
        types::{DbusContext, TData},
    },
    engine::Engine,
};

mod manager_3_0;
pub mod prop_conv;
mod report_3_0;
mod shared;

pub fn get_base_tree<'a, E>(
    dbus_context: DbusContext<E>,
) -> (Tree<MTSync<TData<E>>, TData<E>>, dbus::Path<'a>)
where
    E: 'static + Engine,
{
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
        .add(
            f.interface("org.freedesktop.DBus.ObjectManager", ())
                .add_m(shared::get_managed_objects_method(&f)),
        )
        .add(
            f.interface(consts::MANAGER_INTERFACE_NAME_3_0, ())
                .add_m(manager_3_0::create_pool_method(&f))
                .add_m(manager_3_0::set_key_method(&f))
                .add_m(manager_3_0::unset_key_method(&f))
                .add_m(manager_3_0::list_keys_method(&f))
                .add_m(manager_3_0::unlock_pool_method(&f))
                .add_m(manager_3_0::destroy_pool_method(&f))
                .add_m(manager_3_0::engine_state_report_method(&f))
                .add_p(manager_3_0::version_property(&f))
                .add_p(manager_3_0::locked_pools_property(&f)),
        )
        .add(
            f.interface(consts::REPORT_INTERFACE_NAME_3_0, ())
                .add_m(report_3_0::get_report_method(&f)),
        );

    let path = obj_path.get_name().to_owned();
    (base_tree.add(obj_path), path)
}

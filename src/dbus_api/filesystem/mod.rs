// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::tree::Factory;

use crate::{
    dbus_api::{
        consts,
        types::{DbusContext, OPContext},
        util::make_object_path,
    },
    engine::{Filesystem, FilesystemUuid, MaybeDbusPath},
};

mod fetch_properties_2_0;
mod filesystem_2_0;
mod shared;

pub fn create_dbus_filesystem<'a>(
    dbus_context: &DbusContext,
    parent: dbus::Path<'static>,
    uuid: FilesystemUuid,
    filesystem: &mut dyn Filesystem,
) -> dbus::Path<'a> {
    let f = Factory::new_fn();

    let object_name = make_object_path(dbus_context);

    let object_path = f
        .object_path(object_name, Some(OPContext::new(parent, uuid)))
        .introspectable()
        .add(
            f.interface(consts::FILESYSTEM_INTERFACE_NAME, ())
                .add_m(filesystem_2_0::rename_method(&f))
                .add_p(filesystem_2_0::devnode_property(&f))
                .add_p(filesystem_2_0::name_property(&f))
                .add_p(filesystem_2_0::pool_property(&f))
                .add_p(filesystem_2_0::uuid_property(&f))
                .add_p(filesystem_2_0::created_property(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME, ())
                .add_m(fetch_properties_2_0::get_all_properties_method(&f))
                .add_m(fetch_properties_2_0::get_properties_method(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME_2_1, ())
                .add_m(fetch_properties_2_0::get_all_properties_method(&f))
                .add_m(fetch_properties_2_0::get_properties_method(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME_2_2, ())
                .add_m(fetch_properties_2_0::get_all_properties_method(&f))
                .add_m(fetch_properties_2_0::get_properties_method(&f)),
        );

    let path = object_path.get_name().to_owned();
    dbus_context.actions.borrow_mut().push_add(object_path);
    filesystem.set_dbus_path(MaybeDbusPath(Some(path.clone())));
    path
}

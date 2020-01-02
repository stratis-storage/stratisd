// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{self, tree::Factory};

use crate::{
    dbus_api::{
        consts,
        filesystem::{
            fetch_properties_2_0::api::{get_all_properties_method, get_properties_method},
            filesystem_2_0::api::{
                created_property, devnode_property, name_property, pool_property, rename_method,
                uuid_property,
            },
        },
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
                .add_m(rename_method(&f))
                .add_p(devnode_property(&f))
                .add_p(name_property(&f))
                .add_p(pool_property(&f))
                .add_p(uuid_property(&f))
                .add_p(created_property(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME, ())
                .add_m(get_all_properties_method(&f))
                .add_m(get_properties_method(&f)),
        );

    let path = object_path.get_name().to_owned();
    dbus_context.actions.borrow_mut().push_add(object_path);
    filesystem.set_dbus_path(MaybeDbusPath(Some(path.clone())));
    path
}

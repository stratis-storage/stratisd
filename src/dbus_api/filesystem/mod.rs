// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, iter::FromIterator};

use chrono::SecondsFormat;
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
    engine::{Filesystem, FilesystemUuid, MaybeDbusPath, Name},
};

mod fetch_properties_2_0;
mod filesystem_2_0;
mod shared;

pub fn create_dbus_filesystem<'a>(
    dbus_context: &DbusContext,
    parent: dbus::Path<'static>,
    pool_name: &Name,
    name: &Name,
    uuid: FilesystemUuid,
    filesystem: &mut dyn Filesystem,
) -> dbus::Path<'a> {
    let f = Factory::new_fn();

    let object_name = make_object_path(dbus_context);

    let object_path = f
        .object_path(
            object_name,
            Some(OPContext::new(
                parent.clone(),
                uuid,
                ObjectPathType::Filesystem,
            )),
        )
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
        );

    let path = object_path.get_name().to_owned();
    let interfaces = get_inital_properties(parent, pool_name, name, uuid, filesystem);
    dbus_context
        .actions
        .borrow_mut()
        .push_add(object_path, interfaces);
    filesystem.set_dbus_path(MaybeDbusPath(Some(path.clone())));
    path
}

/// Get the initial state of all properties associated with a filesystem object.
pub fn get_inital_properties(
    parent: dbus::Path<'static>,
    pool_name: &Name,
    fs_name: &Name,
    fs_uuid: FilesystemUuid,
    fs: &dyn Filesystem,
) -> HashMap<String, HashMap<String, Variant<Box<dyn RefArg>>>> {
    let fs_iface_map = HashMap::from_iter(
        vec![
            (
                consts::FILESYSTEM_NAME_PROP,
                Variant(Box::new(fs_name.to_string()) as Box<dyn RefArg>),
            ),
            (
                consts::FILESYSTEM_UUID_PROP,
                Variant(Box::new(fs_uuid.to_simple_ref().to_string()) as Box<dyn RefArg>),
            ),
            (
                consts::FILESYSTEM_DEVNODE_PROP,
                Variant(Box::new(
                    fs.path_to_mount_filesystem(pool_name, fs_name)
                        .display()
                        .to_string(),
                ) as Box<dyn RefArg>),
            ),
            (
                consts::FILESYSTEM_POOL_PROP,
                Variant(Box::new(parent) as Box<dyn RefArg>),
            ),
            (
                consts::FILESYSTEM_CREATED_PROP,
                Variant(
                    Box::new(fs.created().to_rfc3339_opts(SecondsFormat::Secs, true))
                        as Box<dyn RefArg>,
                ),
            ),
        ]
        .into_iter()
        .map(|(key, val)| (key.to_string(), val)),
    );

    HashMap::from_iter(
        vec![
            (consts::FILESYSTEM_INTERFACE_NAME, fs_iface_map),
            (consts::PROPERTY_FETCH_INTERFACE_NAME, HashMap::new()),
            (consts::PROPERTY_FETCH_INTERFACE_NAME_2_1, HashMap::new()),
        ]
        .into_iter()
        .map(|(key, val)| (key.to_string(), val)),
    )
}

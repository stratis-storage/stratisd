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
    engine::{Engine, FilesystemUuid, Name, Pool, StratisUuid},
};

mod filesystem_3_0;
pub mod prop_conv;
mod shared;

pub fn create_dbus_filesystem<'a, E>(
    dbus_context: &DbusContext<E>,
    parent: dbus::Path<'static>,
    pool_name: &Name,
    name: &Name,
    uuid: FilesystemUuid,
    filesystem: &<E::Pool as Pool>::Filesystem,
) -> dbus::Path<'a>
where
    E: 'static + Engine,
{
    let f = Factory::new_sync();

    let object_name = make_object_path(dbus_context);

    let object_path = f
        .object_path(
            object_name,
            Some(OPContext::new(parent.clone(), StratisUuid::Fs(uuid))),
        )
        .introspectable()
        .add(
            f.interface(consts::FILESYSTEM_INTERFACE_NAME_3_0, ())
                .add_m(filesystem_3_0::rename_method(&f))
                .add_p(filesystem_3_0::devnode_property(&f))
                .add_p(filesystem_3_0::name_property(&f))
                .add_p(filesystem_3_0::pool_property(&f))
                .add_p(filesystem_3_0::uuid_property(&f))
                .add_p(filesystem_3_0::created_property(&f))
                .add_p(filesystem_3_0::size_property(&f))
                .add_p(filesystem_3_0::used_property(&f)),
        )
        .add(
            f.interface(consts::FILESYSTEM_INTERFACE_NAME_3_1, ())
                .add_m(filesystem_3_0::rename_method(&f))
                .add_p(filesystem_3_0::devnode_property(&f))
                .add_p(filesystem_3_0::name_property(&f))
                .add_p(filesystem_3_0::pool_property(&f))
                .add_p(filesystem_3_0::uuid_property(&f))
                .add_p(filesystem_3_0::created_property(&f))
                .add_p(filesystem_3_0::size_property(&f))
                .add_p(filesystem_3_0::used_property(&f)),
        )
        .add(
            f.interface(consts::FILESYSTEM_INTERFACE_NAME_3_2, ())
                .add_m(filesystem_3_0::rename_method(&f))
                .add_p(filesystem_3_0::devnode_property(&f))
                .add_p(filesystem_3_0::name_property(&f))
                .add_p(filesystem_3_0::pool_property(&f))
                .add_p(filesystem_3_0::uuid_property(&f))
                .add_p(filesystem_3_0::created_property(&f))
                .add_p(filesystem_3_0::size_property(&f))
                .add_p(filesystem_3_0::used_property(&f)),
        );

    let path = object_path.get_name().to_owned();
    let interfaces = get_fs_properties::<E>(parent, pool_name, name, uuid, filesystem);
    dbus_context.push_add(object_path, interfaces);
    path
}

/// Get the initial state of all properties associated with a filesystem object.
pub fn get_fs_properties<E>(
    parent: dbus::Path<'static>,
    pool_name: &Name,
    fs_name: &Name,
    fs_uuid: FilesystemUuid,
    fs: &<E::Pool as Pool>::Filesystem,
) -> InterfacesAddedThreadSafe
where
    E: 'static + Engine,
{
    initial_properties! {
        consts::FILESYSTEM_INTERFACE_NAME_3_0 => {
            consts::FILESYSTEM_NAME_PROP => shared::fs_name_prop(fs_name),
            consts::FILESYSTEM_UUID_PROP => uuid_to_string!(fs_uuid),
            consts::FILESYSTEM_DEVNODE_PROP => shared::fs_devnode_prop::<E>(fs, pool_name, fs_name),
            consts::FILESYSTEM_POOL_PROP => parent.clone(),
            consts::FILESYSTEM_CREATED_PROP => shared::fs_created_prop::<E>(fs),
            consts::FILESYSTEM_SIZE_PROP => shared::fs_size_prop(fs),
            consts::FILESYSTEM_USED_PROP => shared::fs_used_prop::<E>(fs)
        },
        consts::FILESYSTEM_INTERFACE_NAME_3_1 => {
            consts::FILESYSTEM_NAME_PROP => shared::fs_name_prop(fs_name),
            consts::FILESYSTEM_UUID_PROP => uuid_to_string!(fs_uuid),
            consts::FILESYSTEM_DEVNODE_PROP => shared::fs_devnode_prop::<E>(fs, pool_name, fs_name),
            consts::FILESYSTEM_POOL_PROP => parent.clone(),
            consts::FILESYSTEM_CREATED_PROP => shared::fs_created_prop::<E>(fs),
            consts::FILESYSTEM_SIZE_PROP => shared::fs_size_prop(fs),
            consts::FILESYSTEM_USED_PROP => shared::fs_used_prop::<E>(fs)
        },
        consts::FILESYSTEM_INTERFACE_NAME_3_2 => {
            consts::FILESYSTEM_NAME_PROP => shared::fs_name_prop(fs_name),
            consts::FILESYSTEM_UUID_PROP => uuid_to_string!(fs_uuid),
            consts::FILESYSTEM_DEVNODE_PROP => shared::fs_devnode_prop::<E>(fs, pool_name, fs_name),
            consts::FILESYSTEM_POOL_PROP => parent,
            consts::FILESYSTEM_CREATED_PROP => shared::fs_created_prop::<E>(fs),
            consts::FILESYSTEM_SIZE_PROP => shared::fs_size_prop(fs),
            consts::FILESYSTEM_USED_PROP => shared::fs_used_prop::<E>(fs)
        }
    }
}

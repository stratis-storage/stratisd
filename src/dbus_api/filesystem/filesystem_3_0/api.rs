// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Access, EmitsChangedSignal, Factory, MTSync, Method, Property};

use crate::dbus_api::{
    consts,
    filesystem::filesystem_3_0::{
        methods::rename_filesystem,
        props::{get_filesystem_created, get_filesystem_devnode, get_filesystem_name},
    },
    types::TData,
    util::{get_parent, get_uuid},
};

pub fn rename_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("SetName", (), rename_filesystem)
        .in_arg(("name", "s"))
        // b: true if UUID of changed resource has been returned
        // s: UUID of changed resource
        //
        // Rust representation: (bool, String)
        .out_arg(("result", "(bs)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn devnode_property(f: &Factory<MTSync<TData>, TData>) -> Property<MTSync<TData>, TData> {
    f.property::<&str, _>(consts::FILESYSTEM_DEVNODE_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Invalidates)
        .on_get(get_filesystem_devnode)
}

pub fn name_property(f: &Factory<MTSync<TData>, TData>) -> Property<MTSync<TData>, TData> {
    f.property::<&str, _>(consts::FILESYSTEM_NAME_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_filesystem_name)
}

pub fn pool_property(f: &Factory<MTSync<TData>, TData>) -> Property<MTSync<TData>, TData> {
    f.property::<&dbus::Path, _>(consts::FILESYSTEM_POOL_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_parent)
}

pub fn uuid_property(f: &Factory<MTSync<TData>, TData>) -> Property<MTSync<TData>, TData> {
    f.property::<&str, _>(consts::FILESYSTEM_UUID_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_uuid)
}

pub fn created_property(f: &Factory<MTSync<TData>, TData>) -> Property<MTSync<TData>, TData> {
    f.property::<&str, _>("Created", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_filesystem_created)
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Access, EmitsChangedSignal, Factory, MTSync, Method, Property};

use crate::dbus_api::{
    consts,
    pool::pool_3_7::{
        methods::{destroy_filesystems, fs_metadata, metadata},
        props::get_pool_metadata_version,
    },
    types::TData,
};

pub fn destroy_filesystems_method(
    f: &Factory<MTSync<TData>, TData>,
) -> Method<MTSync<TData>, TData> {
    f.method("DestroyFilesystems", (), destroy_filesystems)
        .in_arg(("filesystems", "ao"))
        // b: true if filesystems were destroyed
        // as: Array of UUIDs of destroyed filesystems
        //
        // Rust representation: (bool, Vec<String>)
        .out_arg(("results", "(bas)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn get_metadata_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("Metadata", (), metadata)
        .in_arg(("current", "b"))
        // A string representing the pool-level metadata in serialized JSON
        // format.
        //
        // Rust representation: String
        .out_arg(("results", "s"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn metadata_version_property(
    f: &Factory<MTSync<TData>, TData>,
) -> Property<MTSync<TData>, TData> {
    f.property::<u64, _>(consts::POOL_METADATA_VERSION_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_pool_metadata_version)
}

pub fn get_fs_metadata_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("FilesystemMetadata", (), fs_metadata)
        .in_arg(("fs_name", "(bs)"))
        .in_arg(("current", "b"))
        // A string representing the pool's filesystem metadata in serialized
        // JSON format.
        //
        // Rust representation: String
        .out_arg(("results", "s"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

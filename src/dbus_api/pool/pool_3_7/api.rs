// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Factory, MTSync, Method};

use crate::dbus_api::{
    pool::pool_3_7::methods::{destroy_filesystems, metadata},
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

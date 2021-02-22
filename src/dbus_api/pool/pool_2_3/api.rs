// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Factory, MTSync, Method};

use crate::dbus_api::{
    pool::pool_2_3::methods::{bind_clevis, unbind_clevis},
    types::TData,
};

pub fn bind_clevis_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("Bind", (), bind_clevis)
        .in_arg(("pin", "s"))
        .in_arg(("json", "s"))
        // b: Indicates if new clevis bindings were added
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn unbind_clevis_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("Unbind", (), unbind_clevis)
        // b: Indicates if clevis bindings were removed
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

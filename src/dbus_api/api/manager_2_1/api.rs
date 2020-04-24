// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::tree::{Factory, MTFn, Method};

use crate::dbus_api::{
    api::manager_2_1::methods::{add_key, create_pool},
    types::TData,
};

pub fn create_pool_method(f: &Factory<MTFn<TData>, TData>) -> Method<MTFn<TData>, TData> {
    f.method("CreatePool", (), create_pool)
        .in_arg(("name", "s"))
        .in_arg(("redundancy", "(bq)"))
        .in_arg(("devices", "as"))
        // Optional key description of key in the kernel keyring
        // b: true if the pool should be encrypted
        // s: key description
        //
        // Rust representation: (bool, String)
        .in_arg(("key_desc", "(bs)"))
        // In order from left to right:
        // b: true if a pool was created and object paths were returned
        // o: Object path for Pool
        // a(o): Array of object paths for block devices
        //
        // Rust representation: (bool, (dbus::Path, Vec<dbus::Path>))
        .out_arg(("result", "(b(oao))"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn add_key_method(f: &Factory<MTFn<TData>, TData>) -> Method<MTFn<TData>, TData> {
    f.method("AddKey", (), add_key)
        .in_arg(("key_desc", "s"))
        .in_arg(("key_fd", "h"))
        // b: true if the key state was changed in the kernel keyring.
        // b: true if the key description already existed in the kernel keyring and
        //    the key data has been changed to a new value.
        //
        // Rust representation: bool
        .out_arg(("result", "(bb)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

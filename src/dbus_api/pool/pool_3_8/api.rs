// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Factory, MTSync, Method};

use crate::dbus_api::{pool::pool_3_8::methods::encrypt_pool, types::TData};

pub fn encrypt_pool_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("EncryptPool", (), encrypt_pool)
        // Optional key description of key in the kernel keyring
        // b: true if the pool should be encrypted and able to be
        // unlocked with a passphrase associated with this key description.
        // s: key description
        //
        // Rust representation: (bool, String)
        .in_arg(("key_desc", "(bs)"))
        // Optional Clevis information for binding on initialization.
        // b: true if the pool should be encrypted and able to be unlocked
        // using Clevis.
        // s: pin name
        // s: JSON config for Clevis use
        //
        // Rust representation: (bool, (String, String))
        .in_arg(("clevis_info", "(b(ss))"))
        // b: true if pool was newly encrypted
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

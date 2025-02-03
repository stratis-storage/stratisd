// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Factory, MTSync, Method};

use crate::dbus_api::{
    pool::pool_3_9::methods::{decrypt_pool, encrypt_pool, reencrypt_pool},
    types::TData,
};

pub fn encrypt_pool_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("EncryptPool", (), encrypt_pool)
        // Optional key descriptions of key in the kernel keyring
        // a: array of zero or more elements
        // b: true if a token slot is specified
        // i: token slot
        // s: key description
        //
        // Rust representation: Vec<((bool, u32), String)>
        .in_arg(("key_descs", "a((bu)s)"))
        // Optional Clevis infos for binding on initialization.
        // a: array of zero or more elements
        // b: true if a token slot is specified
        // i: token slot
        // s: pin name
        // s: JSON config for Clevis use
        //
        // Rust representation: Vec<((bool, u32), String, String)>
        .in_arg(("clevis_infos", "a((bu)ss)"))
        // b: true if pool was newly encrypted
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn reencrypt_pool_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("ReencryptPool", (), reencrypt_pool)
        // b: true if successful
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn decrypt_pool_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("DecryptPool", (), decrypt_pool)
        // b: true if successful
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

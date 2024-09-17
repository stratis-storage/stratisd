// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::arg::{RefArg, Variant};
use dbus_tree::{Access, EmitsChangedSignal, Factory, MTSync, Method, Property};

use crate::dbus_api::{
    pool::{
        consts,
        pool_3_8::{
            methods::{
                bind_clevis, bind_keyring, encrypt_pool, rebind_clevis, rebind_keyring,
                reencrypt_pool, unbind_clevis, unbind_keyring,
            },
            props::{get_pool_clevis_infos, get_pool_key_descs},
        },
    },
    types::TData,
};

pub fn bind_clevis_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("BindClevis", (), bind_clevis)
        .in_arg(("pin", "s"))
        .in_arg(("json", "s"))
        .in_arg(("token_slot", "(bu)"))
        // b: Indicates if new clevis bindings were added
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn unbind_clevis_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("UnbindClevis", (), unbind_clevis)
        .in_arg(("token_slot", "(bu)"))
        // b: Indicates if clevis bindings were removed
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn bind_keyring_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("BindKeyring", (), bind_keyring)
        .in_arg(("key_desc", "s"))
        .in_arg(("token_slot", "(bu)"))
        // b: Indicates if new keyring bindings were added
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn unbind_keyring_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("UnbindKeyring", (), unbind_keyring)
        .in_arg(("token_slot", "(bu)"))
        // b: Indicates if keyring bindings were removed
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn rebind_keyring_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("RebindKeyring", (), rebind_keyring)
        .in_arg(("key_desc", "s"))
        .in_arg(("token_slot", "(bu)"))
        // b: Indicates if keyring bindings were changed
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn rebind_clevis_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("RebindClevis", (), rebind_clevis)
        .in_arg(("token_slot", "(bu)"))
        // b: Indicates if Clevis bindings were changed
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn key_descs_property(f: &Factory<MTSync<TData>, TData>) -> Property<MTSync<TData>, TData> {
    f.property::<Variant<Box<dyn RefArg>>, _>(consts::POOL_KEY_DESCS_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_pool_key_descs)
}

pub fn clevis_infos_property(f: &Factory<MTSync<TData>, TData>) -> Property<MTSync<TData>, TData> {
    f.property::<Variant<Box<dyn RefArg>>, _>(consts::POOL_CLEVIS_INFOS_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_pool_clevis_infos)
}

pub fn encrypt_pool_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("EncryptPool", (), encrypt_pool)
        // Optional key descriptions of key in the kernel keyring
        // a: array of zero or more elements
        // b: true if a token slot is specified
        // i: token slot
        // s: key description
        //
        // Rust representation: Vec<((bool, u32), String)>
        .in_arg(("key_desc", "a((bu)s)"))
        // Optional Clevis infos for binding on initialization.
        // a: array of zero or more elements
        // b: true if a token slot is specified
        // i: token slot
        // s: pin name
        // s: JSON config for Clevis use
        //
        // Rust representation: Vec<((bool, u32), String, String)>
        .in_arg(("clevis_info", "a((bu)ss)"))
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

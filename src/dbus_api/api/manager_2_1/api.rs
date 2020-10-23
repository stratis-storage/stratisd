// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::tree::{Factory, MTSync, Method};

use crate::dbus_api::{
    api::manager_2_1::methods::{create_pool, set_key, unlock_pool, unset_key},
    types::TData,
};

pub fn create_pool_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
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

pub fn set_key_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("SetKey", (), set_key)
        .in_arg(("key_desc", "s"))
        .in_arg(("key_fd", "h"))
        .in_arg(("interactive", "b"))
        // b: true if the key state was changed in the kernel keyring.
        // b: true if the key description already existed in the kernel keyring and
        //    the key data has been changed to a new value.
        //
        // Rust representation: (bool, bool)
        .out_arg(("result", "(bb)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn unset_key_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("UnsetKey", (), unset_key)
        .in_arg(("key_desc", "s"))
        // b: true if the key was unset from the keyring. false if the key
        //    was not present in the keyring before the operation.
        //
        // Rust representation: bool
        .out_arg(("result", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn unlock_pool_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("UnlockPool", (), unlock_pool)
        .in_arg(("pool_uuid", "s"))
        // b: true if some encrypted devices were newly opened.
        // as: array of device UUIDs converted to Strings of all of the newly opened
        //     devices.
        //
        // Rust representation: (bool, Vec<DevUuid>)
        .out_arg(("result", "(bas)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

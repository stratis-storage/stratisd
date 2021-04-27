// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Factory, MTSync, Method};

use crate::dbus_api::{
    api::manager_2_4::methods::{create_pool, engine_state_report},
    types::TData,
};

pub fn engine_state_report_method(
    f: &Factory<MTSync<TData>, TData>,
) -> Method<MTSync<TData>, TData> {
    f.method("EngineStateReport", (), engine_state_report)
        // s: JSON engine state report as a string.
        //
        // Rust representation: Value
        .out_arg(("result", "s"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn create_pool_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("CreatePool", (), create_pool)
        .in_arg(("name", "s"))
        .in_arg(("redundancy", "(bq)"))
        .in_arg(("devices", "as"))
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

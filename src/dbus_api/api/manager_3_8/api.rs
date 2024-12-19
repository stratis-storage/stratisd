// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Access, EmitsChangedSignal, Factory, MTSync, Method, Property};

use crate::dbus_api::{
    api::{
        manager_3_8::{
            methods::{create_pool, start_pool},
            props::get_stopped_pools,
        },
        prop_conv::StoppedOrLockedPools,
    },
    consts,
    types::TData,
};

pub fn start_pool_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("StartPool", (), start_pool)
        .in_arg(("id", "s"))
        .in_arg(("id_type", "s"))
        .in_arg(("unlock_method", "(bs)"))
        .in_arg(("key_fd", "(bh)"))
        // In order from left to right:
        // b: true if the pool was newly started
        // o: pool path
        // oa: block device paths
        // oa: filesystem paths
        //
        // Rust representation: bool
        .out_arg(("result", "(b(oaoao))"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn create_pool_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("CreatePool", (), create_pool)
        .in_arg(("name", "s"))
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
        // Optional journal size for integrity metadata reservation.
        // b: true if the size should be specified.
        //    false if the default should be used.
        // i: Integer representing journal size in bytes.
        //
        // Rust representation: (bool, u64)
        .in_arg(("journal_size", "(bt)"))
        // Optional tag size or specification for integrity metadata
        // reservation.
        // b: true if the size should be specified.
        //    false if the default should be used.
        // s: Tag size specification.
        //
        // Rust representation: (bool, String)
        .in_arg(("tag_spec", "(bs)"))
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

pub fn stopped_pools_property(f: &Factory<MTSync<TData>, TData>) -> Property<MTSync<TData>, TData> {
    f.property::<StoppedOrLockedPools, _>(consts::STOPPED_POOLS_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_stopped_pools)
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Access, EmitsChangedSignal, Factory, MTSync, Method, Property};

use crate::dbus_api::{
    api::{
        manager_3_2::{
            methods::{refresh_state, start_pool, stop_pool},
            props::get_stopped_pools,
        },
        prop_conv::StoppedOrLockedPools,
    },
    consts,
    types::TData,
};

pub fn start_pool_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("StartPool", (), start_pool)
        .in_arg(("pool_uuid", "s"))
        .in_arg(("unlock_method", "(bs)"))
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

pub fn stop_pool_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("StopPool", (), stop_pool)
        .in_arg(("pool", "o"))
        // In order from left to right:
        // b: true if the pool was newly stopped
        // s: string representation of UUID of stopped pool
        //
        // Rust representation: (bool, String)
        .out_arg(("result", "(bs)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn refresh_state_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("RefreshState", (), refresh_state)
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn stopped_pools_property(f: &Factory<MTSync<TData>, TData>) -> Property<MTSync<TData>, TData> {
    f.property::<StoppedOrLockedPools, _>(consts::STOPPED_POOLS_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_stopped_pools)
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::tree::{Factory, MTFn, Method};

use crate::dbus_api::{
    pool::pool_2_1::methods::{add_cachedevs, init_cache},
    types::TData,
};

pub fn init_cache_method(f: &Factory<MTFn<TData>, TData>) -> Method<MTFn<TData>, TData> {
    f.method("InitCache", (), init_cache)
        .in_arg(("devices", "as"))
        // b: Indicates if any cache devices were added
        // ao: Array of object paths of created cache devices
        //
        // Rust representation: (bool, Vec<dbus::path>)
        .out_arg(("results", "(bao)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn add_cachedevs_method(f: &Factory<MTFn<TData>, TData>) -> Method<MTFn<TData>, TData> {
    f.method("AddCacheDevs", (), add_cachedevs)
        .in_arg(("devices", "as"))
        // b: Indicates if any cache devices were added
        // ao: Array of object paths of created cache devices
        //
        // Rust representation: (bool, Vec<dbus::path>)
        .out_arg(("results", "(bao)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

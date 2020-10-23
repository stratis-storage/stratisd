// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::tree::{Access, EmitsChangedSignal, Factory, MTSync, Method, Property};

use crate::dbus_api::{
    api::manager_2_0::{
        methods::{configure_simulator, create_pool, destroy_pool},
        props::get_version,
    },
    types::TData,
};

pub fn create_pool_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("CreatePool", (), create_pool)
        .in_arg(("name", "s"))
        .in_arg(("redundancy", "(bq)"))
        .in_arg(("devices", "as"))
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

pub fn destroy_pool_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("DestroyPool", (), destroy_pool)
        .in_arg(("pool", "o"))
        // In order from left to right:
        // b: true if a valid UUID is returned - otherwise no action was performed
        // s: String representation of pool UUID that was destroyed
        //
        // Rust representation: (bool, String)
        .out_arg(("result", "(bs)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn configure_simulator_method(
    f: &Factory<MTSync<TData>, TData>,
) -> Method<MTSync<TData>, TData> {
    f.method("ConfigureSimulator", (), configure_simulator)
        .in_arg(("denominator", "u"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn version_property(f: &Factory<MTSync<TData>, TData>) -> Property<MTSync<TData>, TData> {
    f.property::<&str, _>("Version", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_version)
}

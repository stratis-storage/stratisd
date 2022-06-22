// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Factory, MTSync, Method};

use crate::{
    dbus_api::{pool::pool_3_3::methods::grow_physical, types::TData},
    engine::Engine,
};

pub fn grow_physical_device_method<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Method<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.method("GrowPhysicalDevice", (), grow_physical)
        // s: String representation of device UUID
        .in_arg(("dev", "s"))
        // b: true if the specified device was newly extended
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

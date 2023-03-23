// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Factory, MTSync, Method};

use crate::{
    dbus_api::{api::manager_3_6::methods::stop_pool, types::TData},
    engine::Engine,
};

pub fn stop_pool_method<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Method<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.method("StopPool", (), stop_pool)
        .in_arg(("id", "s"))
        .in_arg(("id_type", "s"))
        // In order from left to right:
        // b: true if the pool was newly stopped
        // s: string representation of UUID of stopped pool
        //
        // Rust representation: (bool, String)
        .out_arg(("result", "(bs)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

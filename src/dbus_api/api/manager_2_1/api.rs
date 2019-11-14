// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::tree::{Factory, MTFn, Method};

use crate::dbus_api::{api::manager_2_1::methods::create_pool, types::TData};

pub fn create_pool_method(f: &Factory<MTFn<TData>, TData>) -> Method<MTFn<TData>, TData> {
    f.method("CreatePool", (), create_pool)
        .in_arg(("name", "s"))
        .in_arg(("redundancy", "(bq)"))
        .in_arg(("devices", "as"))
        // Optional keyfile
        // b: true if the pool should be encrypted
        // s: Path to keyfile
        //
        // Rust representation: (bool, String)
        .in_arg(("keyfile", "(bs)"))
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

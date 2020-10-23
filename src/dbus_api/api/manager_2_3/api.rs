// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::tree::{Factory, MTSync, Method};

use crate::dbus_api::{api::manager_2_3::methods::unlock_pool, types::TData};

pub fn unlock_pool_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("UnlockPool", (), unlock_pool)
        .in_arg(("pool_uuid", "s"))
        .in_arg(("unlock_method", "s"))
        // b: true if some encrypted devices were newly opened.
        // as: array of device UUIDs converted to Strings of all of the newly opened
        //     devices.
        //
        // Rust representation: (bool, Vec<DevUuid>)
        .out_arg(("result", "(bas)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

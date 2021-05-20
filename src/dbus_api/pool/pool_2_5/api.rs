// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Factory, MTSync, Method};

use crate::dbus_api::{pool::pool_2_5::methods::rebind_keyring, types::TData};

pub fn rebind_keyring_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("RebindKeyring", (), rebind_keyring)
        .in_arg(("key_desc", "s"))
        // b: Indicates if keyring bindings were changed
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

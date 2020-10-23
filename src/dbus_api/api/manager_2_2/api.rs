// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::tree::{Factory, MTSync, Method};

use crate::dbus_api::{api::manager_2_2::methods::set_key, types::TData};

pub fn set_key_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("SetKey", (), set_key)
        .in_arg(("key_desc", "s"))
        .in_arg(("key_fd", "h"))
        .in_arg(("interactive", "b"))
        // b: true if the key state was changed in the kernel keyring.
        // b: true if the key description already existed in the kernel keyring and
        //    the key data has been changed to a new value.
        //
        // Rust representation: (bool, bool)
        .out_arg(("result", "(bb)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

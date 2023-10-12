// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Factory, MTSync, Method};

use crate::dbus_api::{pool::pool_3_6::methods::create_filesystems, types::TData};

pub fn create_filesystems_method(
    f: &Factory<MTSync<TData>, TData>,
) -> Method<MTSync<TData>, TData> {
    f.method("CreateFilesystems", (), create_filesystems)
        .in_arg(("specs", "a(s(bs)(bs))"))
        // b: true if filesystems were created
        // a(os): Array of tuples with object paths and names
        //
        // Rust representation: (bool, Vec<(dbus::Path, String)>)
        .out_arg(("results", "(ba(os))"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

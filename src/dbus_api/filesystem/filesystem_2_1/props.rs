// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{
    arg::IterAppend,
    tree::{MTFn, MethodErr, PropInfo},
};

use crate::dbus_api::{
    filesystem::shared::{self, get_filesystem_property},
    types::TData,
};

/// Get the devnode for an object path.
pub fn get_filesystem_devnode(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_filesystem_property(i, p, |(pool_name, fs_name, fs)| {
        Ok(shared::fs_devnode_prop(fs, &pool_name, &fs_name))
    })
}

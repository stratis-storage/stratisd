// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::arg::IterAppend;
use dbus_tree::{MTSync, MethodErr, PropInfo};

use crate::dbus_api::{filesystem::shared, types::TData};

/// Get the devnode for an object path.
pub fn get_filesystem_devnode(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    shared::get_filesystem_property(i, p, |(pool_name, fs_name, fs)| {
        Ok(shared::fs_devnode_prop(fs, &pool_name, &fs_name))
    })
}

pub fn get_filesystem_name(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    shared::get_filesystem_property(i, p, |(_, fs_name, _)| Ok(shared::fs_name_prop(&fs_name)))
}

/// Get the creation date and time in rfc3339 format.
pub fn get_filesystem_created(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    shared::get_filesystem_property(i, p, |(_, _, fs)| Ok(shared::fs_created_prop(fs)))
}

/// Get the size of the filesystem in bytes.
pub fn get_filesystem_size(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    shared::get_filesystem_property(i, p, |(_, _, fs)| Ok(shared::fs_size_prop(fs)))
}

/// Get the size of the used portion of the filesystem in bytes.
pub fn get_filesystem_used(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    shared::get_filesystem_property(i, p, |(_, _, fs)| Ok(shared::fs_used_prop(fs)))
}

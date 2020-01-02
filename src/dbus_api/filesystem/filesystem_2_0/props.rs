// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use chrono::SecondsFormat;
use dbus::{
    self,
    arg::IterAppend,
    tree::{MTFn, MethodErr, PropInfo},
};

use crate::{
    dbus_api::{filesystem::shared::filesystem_operation, types::TData},
    engine::{Filesystem, Name},
};

/// Get a filesystem property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to a
/// Filesystem and obtains the property from the filesystem.
fn get_filesystem_property<F, R>(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
    getter: F,
) -> Result<(), MethodErr>
where
    F: Fn((Name, Name, &dyn Filesystem)) -> Result<R, String>,
    R: dbus::arg::Append,
{
    i.append(
        filesystem_operation(p.tree, p.path.get_name(), getter)
            .map_err(|ref e| MethodErr::failed(e))?,
    );
    Ok(())
}

/// Get the devnode for an object path.
pub fn get_filesystem_devnode(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_filesystem_property(i, p, |(pool_name, fs_name, fs)| {
        Ok(fs
            .path_to_mount_filesystem(&pool_name, &fs_name)
            .display()
            .to_string())
    })
}

pub fn get_filesystem_name(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_filesystem_property(i, p, |(_, fs_name, _)| Ok(fs_name.to_owned()))
}

/// Get the creation date and time in rfc3339 format.
pub fn get_filesystem_created(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_filesystem_property(i, p, |(_, _, fs)| {
        Ok(fs.created().to_rfc3339_opts(SecondsFormat::Secs, true))
    })
}

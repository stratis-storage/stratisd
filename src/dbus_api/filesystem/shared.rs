// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use chrono::SecondsFormat;
use dbus::{
    tree::{MTSync, Tree},
    Path,
};

use crate::{
    dbus_api::types::TData,
    engine::{Filesystem, Name},
};

/// Get execute a given closure providing a filesystem object and return
/// the calculated value
pub fn filesystem_operation<F, R>(
    tree: &Tree<MTSync<TData>, TData>,
    object_path: &Path<'static>,
    closure: F,
) -> Result<R, String>
where
    F: Fn((Name, Name, &dyn Filesystem)) -> Result<R, String>,
    R: dbus::arg::Append,
{
    let dbus_context = tree.get_data();

    let filesystem_path = tree
        .get(object_path)
        .expect("tree must contain implicit argument");

    let filesystem_data = filesystem_path
        .get_data()
        .as_ref()
        .ok_or_else(|| format!("no data for object path {}", object_path))?;

    let pool_path = tree
        .get(&filesystem_data.parent)
        .ok_or_else(|| format!("no path for parent object path {}", &filesystem_data.parent))?;

    let pool_uuid = pool_path
        .get_data()
        .as_ref()
        .ok_or_else(|| format!("no data for object path {}", object_path))?
        .uuid;

    let mutex_lock = mutex_lock!(dbus_context.engine, |e| e.to_string());
    let (pool_name, pool) = (*mutex_lock)
        .get_pool(pool_uuid)
        .ok_or_else(|| format!("no pool corresponding to uuid {}", &pool_uuid))?;
    let filesystem_uuid = filesystem_data.uuid;
    let (fs_name, fs) = pool
        .get_filesystem(filesystem_uuid)
        .ok_or_else(|| format!("no name for filesystem with uuid {}", &filesystem_uuid))?;
    closure((pool_name, fs_name, fs))
}

/// Generate D-Bus representation of name property.
#[inline]
pub fn fs_name_prop(name: &Name) -> String {
    name.to_owned()
}

/// Generate D-Bus representation of devnode property.
#[inline]
pub fn fs_devnode_prop(fs: &dyn Filesystem, pool_name: &Name, fs_name: &Name) -> String {
    fs.path_to_mount_filesystem(pool_name, fs_name)
        .display()
        .to_string()
}

/// Generate D-Bus representation of created property.
#[inline]
pub fn fs_created_prop(fs: &dyn Filesystem) -> String {
    fs.created().to_rfc3339_opts(SecondsFormat::Secs, true)
}

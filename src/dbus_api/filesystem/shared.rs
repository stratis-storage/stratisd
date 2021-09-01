// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use chrono::SecondsFormat;
use dbus::Path;
use dbus_tree::{MTSync, Tree};

use crate::{
    dbus_api::types::TData,
    engine::{Engine, Filesystem, Name, Pool},
};

/// Get execute a given closure providing a filesystem object and return
/// the calculated value
pub fn filesystem_operation<F, R, E>(
    tree: &Tree<MTSync<TData<E>>, TData<E>>,
    object_path: &Path<'static>,
    closure: F,
) -> Result<R, String>
where
    F: Fn((Name, Name, &<E::Pool as Pool>::Filesystem)) -> Result<R, String>,
    R: dbus::arg::Append,
    E: Engine,
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

    let pool_uuid = typed_uuid_string_err!(
        pool_path
            .get_data()
            .as_ref()
            .ok_or_else(|| format!("no data for object path {}", object_path))?
            .uuid;
        Pool
    );

    let mutex_lock = dbus_context.engine.blocking_lock();
    let (pool_name, pool) = mutex_lock
        .get_pool(pool_uuid)
        .ok_or_else(|| format!("no pool corresponding to uuid {}", &pool_uuid))?;
    let filesystem_uuid = typed_uuid_string_err!(filesystem_data.uuid; Fs);
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
pub fn fs_devnode_prop<E>(
    fs: &<E::Pool as Pool>::Filesystem,
    pool_name: &Name,
    fs_name: &Name,
) -> String
where
    E: Engine,
{
    fs.path_to_mount_filesystem(pool_name, fs_name)
        .display()
        .to_string()
}

/// Generate D-Bus representation of created property.
#[inline]
pub fn fs_created_prop<E>(fs: &<E::Pool as Pool>::Filesystem) -> String
where
    E: Engine,
{
    fs.created().to_rfc3339_opts(SecondsFormat::Secs, true)
}

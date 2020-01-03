// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{
    self,
    tree::{MTFn, Tree},
    Path,
};

use crate::{
    dbus_api::types::TData,
    engine::{Filesystem, Name},
};

/// Get execute a given closure providing a filesystem object and return
/// the calculated value
pub fn filesystem_operation<F, R>(
    tree: &Tree<MTFn<TData>, TData>,
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

    let engine = dbus_context.engine.borrow();
    let (pool_name, pool) = engine
        .get_pool(pool_uuid)
        .ok_or_else(|| format!("no pool corresponding to uuid {}", &pool_uuid))?;
    let filesystem_uuid = filesystem_data.uuid;
    let (fs_name, fs) = pool
        .get_filesystem(filesystem_uuid)
        .ok_or_else(|| format!("no name for filesystem with uuid {}", &filesystem_uuid))?;
    closure((pool_name, fs_name, fs))
}

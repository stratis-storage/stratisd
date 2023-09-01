// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use chrono::SecondsFormat;
use dbus::{arg::IterAppend, Path};
use dbus_tree::{MTSync, MethodErr, PropInfo, Tree};
use futures::executor::block_on;

use devicemapper::{Bytes, Sectors};

use crate::{
    dbus_api::{filesystem::prop_conv, types::TData},
    engine::{Filesystem, FilesystemUuid, Name, Pool, PoolIdentifier, PropChangeAction, ToDisplay},
};

/// Get execute a given closure providing a filesystem object and return
/// the calculated value
pub fn filesystem_get_operation<F, R>(
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
        .ok_or_else(|| format!("no data for object path {object_path}"))?;

    let pool_path = tree
        .get(&filesystem_data.parent)
        .ok_or_else(|| format!("no path for parent object path {}", &filesystem_data.parent))?;

    let pool_uuid = typed_uuid_string_err!(
        pool_path
            .get_data()
            .as_ref()
            .ok_or_else(|| format!("no data for object path {object_path}"))?
            .uuid;
        Pool
    );

    let guard = block_on(
        dbus_context
            .engine
            .get_pool(PoolIdentifier::Uuid(pool_uuid)),
    )
    .ok_or_else(|| format!("no pool corresponding to uuid {}", &pool_uuid))?;
    let (pool_name, _, pool) = guard.as_tuple();
    let filesystem_uuid = typed_uuid_string_err!(filesystem_data.uuid; Fs);
    let (fs_name, fs) = pool
        .get_filesystem(filesystem_uuid)
        .ok_or_else(|| format!("no name for filesystem with uuid {}", &filesystem_uuid))?;
    closure((pool_name, fs_name, fs))
}

/// Get a filesystem property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to a
/// Filesystem and obtains the property from the filesystem.
pub fn get_filesystem_property<F, R>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
    getter: F,
) -> Result<(), MethodErr>
where
    F: Fn((Name, Name, &dyn Filesystem)) -> Result<R, String>,
    R: dbus::arg::Append,
{
    i.append(
        filesystem_get_operation(p.tree, p.path.get_name(), getter)
            .map_err(|ref e| MethodErr::failed(e))?,
    );
    Ok(())
}

/// Set a filesystem property. The property is set by means of
/// the setter method which takes a mutable reference to a
/// Pool for MDV update purposes.
pub fn fs_set_operation<F, R>(
    tree: &Tree<MTSync<TData>, TData>,
    object_path: &dbus::Path<'static>,
    closure: F,
) -> Result<R, String>
where
    F: Fn((Name, FilesystemUuid, &mut dyn Pool)) -> Result<R, String>,
{
    let dbus_context = tree.get_data();

    let fs_path = tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let fs_uuid = typed_uuid_string_err!(
        fs_path
            .get_data()
            .as_ref()
            .ok_or_else(|| format!("no data for object path {object_path}"))?
            .uuid;
        Fs
    );
    let pool_path = tree
        .get(
            &fs_path
                .get_data()
                .as_ref()
                .ok_or_else(|| format!("no data for object path {object_path}"))?
                .parent,
        )
        .ok_or_else(|| "Parent not found in tree".to_string())?;
    let pool_uuid = typed_uuid_string_err!(
        pool_path
            .get_data()
            .as_ref()
            .ok_or_else(|| format!("no data for object path {object_path}"))?
            .uuid;
        Pool
    );

    let mut guard = block_on(
        dbus_context
            .engine
            .get_mut_pool(PoolIdentifier::Uuid(pool_uuid)),
    )
    .ok_or_else(|| format!("no pool corresponding to uuid {pool_uuid}"))?;
    let (name, _) = guard
        .get_filesystem(fs_uuid)
        .ok_or_else(|| format!("no filesystem corresponding to uuid {fs_uuid}"))?;

    closure((name, fs_uuid, &mut *guard))
}

/// Set a filesystem property. The property is found by means of the setter method which
/// takes a mutable reference to a Filesystem and sets the property on the filesystem.
pub fn set_fs_property_to_display<F, R>(
    p: &PropInfo<'_, MTSync<TData>, TData>,
    prop_name: &str,
    setter: F,
) -> Result<R, MethodErr>
where
    F: Fn((Name, FilesystemUuid, &mut dyn Pool)) -> Result<R, String>,
    R: ToDisplay,
{
    info!("Setting property {}", prop_name);
    let res =
        fs_set_operation(p.tree, p.path.get_name(), setter).map_err(|ref e| MethodErr::failed(e));
    let display = res.as_ref().map(|r| r.to_display());
    let _ = handle_action!(display);
    res
}

/// Get the filesystem size limit for a given filesystem.
#[inline]
pub fn fs_size_limit_prop(fs: &dyn Filesystem) -> (bool, String) {
    prop_conv::fs_size_limit_to_prop(fs.size_limit())
}

/// Set the filesystem size limit for a given filesystem.
#[inline]
pub fn set_fs_size_limit_prop(
    uuid: FilesystemUuid,
    pool: &mut dyn Pool,
    limit: Option<Bytes>,
) -> Result<PropChangeAction<Option<Sectors>>, String> {
    pool.set_fs_size_limit(uuid, limit)
        .map_err(|e| e.to_string())
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

/// Generate D-Bus representation of filesystem size property.
pub fn fs_size_prop(fs: &dyn Filesystem) -> String {
    prop_conv::fs_size_to_prop(fs.size())
}

/// Generate D-Bus representation of used property.
pub fn fs_used_prop(fs: &dyn Filesystem) -> (bool, String) {
    prop_conv::fs_used_to_prop(fs.used().ok())
}

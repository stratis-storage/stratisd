// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use chrono::SecondsFormat;
use dbus::{
    arg::{IterAppend, Variant},
    ffidisp::stdintf::org_freedesktop_dbus::PropertiesPropertiesChanged,
    tree::{MTFn, MethodErr, PropInfo, Tree},
    Path,
};

use crate::{
    dbus_api::{consts, types::TData},
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

/// Get a filesystem property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to a
/// Filesystem and obtains the property from the filesystem.
pub fn get_filesystem_property<F, R>(
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

/// Get the property changed object that should be constructed if a filesystem
/// name is changed.
pub fn get_name_change_properties(
    pool_name: &Name,
    fs_name: &Name,
    fs: &dyn Filesystem,
) -> Vec<PropertiesPropertiesChanged> {
    let mut properties_changed: Vec<PropertiesPropertiesChanged> = vec![];
    let mut r0_properties = PropertiesPropertiesChanged::default();
    r0_properties.changed_properties.insert(
        consts::FILESYSTEM_NAME_PROP.into(),
        Variant(Box::new(fs_name_prop(fs_name))),
    );
    r0_properties.interface_name = consts::FILESYSTEM_INTERFACE_NAME.into();
    properties_changed.push(r0_properties);

    let mut r2_properties = PropertiesPropertiesChanged::default();
    r2_properties.changed_properties.insert(
        consts::FILESYSTEM_NAME_PROP.into(),
        Variant(Box::new(fs_name_prop(fs_name))),
    );
    r2_properties.changed_properties.insert(
        consts::FILESYSTEM_DEVNODE_PROP.into(),
        Variant(Box::new(fs_devnode_prop(fs, pool_name, fs_name))),
    );
    r2_properties.interface_name = consts::FILESYSTEM_INTERFACE_NAME_2_2.into();
    properties_changed.push(r2_properties);

    properties_changed
}

/// Get the property changed object that should be constructed if a filesystem
/// devnode is changed.
pub fn get_devnode_change_properties(
    pool_name: &Name,
    fs_name: &Name,
    fs: &dyn Filesystem,
) -> Vec<PropertiesPropertiesChanged> {
    let mut properties_changed: Vec<PropertiesPropertiesChanged> = vec![];

    let mut r2_properties = PropertiesPropertiesChanged::default();
    r2_properties.changed_properties.insert(
        consts::FILESYSTEM_DEVNODE_PROP.into(),
        Variant(Box::new(fs_devnode_prop(fs, pool_name, fs_name))),
    );
    r2_properties.interface_name = consts::FILESYSTEM_INTERFACE_NAME_2_2.into();
    properties_changed.push(r2_properties);

    properties_changed
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::arg::IterAppend;
use dbus_tree::{MTSync, MethodErr, PropInfo};

use crate::{
    dbus_api::{
        filesystem::shared::{self, filesystem_operation},
        types::TData,
    },
    engine::{Engine, Name, Pool},
};

/// Get a filesystem property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to a
/// Filesystem and obtains the property from the filesystem.
fn get_filesystem_property<F, R, E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
    getter: F,
) -> Result<(), MethodErr>
where
    F: Fn((Name, Name, &<E::Pool as Pool>::Filesystem)) -> Result<R, String>,
    R: dbus::arg::Append,
    E: Engine,
{
    i.append(
        filesystem_operation(p.tree, p.path.get_name(), getter)
            .map_err(|ref e| MethodErr::failed(e))?,
    );
    Ok(())
}

/// Get the devnode for an object path.
pub fn get_filesystem_devnode<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: Engine,
{
    get_filesystem_property(i, p, |(pool_name, fs_name, fs)| {
        Ok(shared::fs_devnode_prop::<E>(fs, &pool_name, &fs_name))
    })
}

pub fn get_filesystem_name<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: Engine,
{
    get_filesystem_property(i, p, |(_, fs_name, _)| Ok(shared::fs_name_prop(&fs_name)))
}

/// Get the creation date and time in rfc3339 format.
pub fn get_filesystem_created<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: Engine,
{
    get_filesystem_property(i, p, |(_, _, fs)| Ok(shared::fs_created_prop::<E>(fs)))
}

/// Get the size of the filesystem in bytes.
pub fn get_filesystem_size<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: Engine,
{
    get_filesystem_property(i, p, |(_, _, fs)| Ok(shared::fs_size_prop(fs)))
}

/// Get the size of the used portion of the filesystem in bytes.
pub fn get_filesystem_used<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: Engine,
{
    get_filesystem_property(i, p, |(_, _, fs)| Ok(shared::fs_used_prop::<E>(fs)))
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use tokio::sync::OwnedRwLockReadGuard;

use chrono::SecondsFormat;
use zbus::{fdo::Error, zvariant::OwnedObjectPath};

use crate::{
    dbus::{util::option_to_tuple, Manager},
    engine::{Filesystem, FilesystemUuid, Name, PoolUuid, SharedGuard},
};

pub fn created_prop(_: Name, _: Name, _: FilesystemUuid, fs: &dyn Filesystem) -> String {
    fs.created().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn devnode_prop(
    pool_name: Name,
    fs_name: Name,
    _: FilesystemUuid,
    fs: &dyn Filesystem,
) -> String {
    fs.path_to_mount_filesystem(&pool_name, &fs_name)
        .display()
        .to_string()
}

pub fn name_prop(_: Name, name: Name, _: FilesystemUuid, _: &dyn Filesystem) -> Name {
    name
}

pub fn pool_prop(
    guard: SharedGuard<OwnedRwLockReadGuard<Manager>>,
    pool_uuid: PoolUuid,
) -> Result<OwnedObjectPath, Error> {
    guard
        .pool_get_path(&pool_uuid)
        .ok_or_else(|| {
            Error::Failed(format!(
                "No object path associated with pool UUID {pool_uuid}"
            ))
        })
        .cloned()
}

pub fn size_prop(_: Name, _: Name, _: FilesystemUuid, fs: &dyn Filesystem) -> String {
    (*fs.size()).to_string()
}

pub fn used_prop(_: Name, _: Name, _: FilesystemUuid, fs: &dyn Filesystem) -> (bool, String) {
    option_to_tuple(fs.used().ok().map(|u| (*u).to_string()), String::new())
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use zbus::Error;

use crate::{
    dbus::util::option_to_tuple,
    engine::{
        Filesystem, FilesystemUuid, Name, Pool, PoolUuid, PropChangeAction, SomeLockWriteGuard,
    },
};

pub fn origin_prop(
    _: Name,
    _: Name,
    _: FilesystemUuid,
    fs: &dyn Filesystem,
) -> (bool, FilesystemUuid) {
    option_to_tuple(fs.origin(), FilesystemUuid::nil())
}

pub fn merge_scheduled_prop(_: Name, _: Name, _: FilesystemUuid, fs: &dyn Filesystem) -> bool {
    fs.merge_scheduled()
}

pub fn set_merge_scheduled_prop(
    guard: &mut SomeLockWriteGuard<PoolUuid, dyn Pool>,
    fs_uuid: FilesystemUuid,
    scheduled: bool,
) -> Result<(), Error> {
    let (_, _, pool) = guard.as_mut_tuple();
    match pool.set_fs_merge_scheduled(fs_uuid, scheduled) {
        Ok(PropChangeAction::NewValue(_v)) => Ok(()),
        Ok(PropChangeAction::Identity) => Ok(()),
        Err(e) => Err(Error::Failure(e.to_string())),
    }
}

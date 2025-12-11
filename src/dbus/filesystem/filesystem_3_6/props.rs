// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::{
    dbus::util::{option_to_tuple, tuple_to_option},
    engine::{
        Filesystem, FilesystemUuid, Name, Pool, PoolUuid, PropChangeAction, SomeLockWriteGuard,
    },
};
use devicemapper::Bytes;
use zbus::Error;

pub fn size_limit_prop(_: Name, _: Name, _: FilesystemUuid, fs: &dyn Filesystem) -> (bool, String) {
    option_to_tuple(
        fs.size_limit().map(|u| (*u.bytes()).to_string()),
        String::new(),
    )
}

pub fn set_size_limit_prop(
    guard: &mut SomeLockWriteGuard<PoolUuid, dyn Pool>,
    fs_uuid: FilesystemUuid,
    size_limit_tuple: (bool, String),
) -> Result<(), Error> {
    let size_limit_str = tuple_to_option(size_limit_tuple);
    let size_limit = match size_limit_str {
        Some(lim) => Some(Bytes(lim.parse::<u128>().map_err(|e| {
            Error::Failure(format!("Failed to parse {lim} as unsigned integer: {e}"))
        })?)),
        None => None,
    };
    let (_, _, pool) = guard.as_mut_tuple();
    match pool.set_fs_size_limit(fs_uuid, size_limit) {
        Ok(PropChangeAction::NewValue(_v)) => Ok(()),
        Ok(PropChangeAction::Identity) => Ok(()),
        Err(e) => Err(Error::Failure(e.to_string())),
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use zbus::Error;

use crate::engine::{Pool, PoolUuid, SomeLockReadGuard, SomeLockWriteGuard};

pub fn fs_limit_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> u64 {
    guard.fs_limit()
}

pub fn set_fs_limit_prop(
    mut guard: SomeLockWriteGuard<PoolUuid, dyn Pool>,
    fs_limit: u64,
) -> Result<(), Error> {
    let (name, uuid, p) = guard.as_mut_tuple();
    p.set_fs_limit(&name, uuid, fs_limit)
        .map_err(|e| Error::Failure(e.to_string()))?;
    Ok(())
}

pub fn enable_overprovisioning_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> bool {
    guard.overprov_enabled()
}

pub fn set_enable_overprovisioning_prop(
    mut guard: SomeLockWriteGuard<PoolUuid, dyn Pool>,
    enable_overprov: bool,
) -> Result<(), Error> {
    let (name, _, p) = guard.as_mut_tuple();
    p.set_overprov_mode(&name, enable_overprov)
        .map_err(|e| Error::Failure(e.to_string()))?;
    Ok(())
}

pub fn no_alloc_space_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> bool {
    guard.out_of_alloc_space()
}

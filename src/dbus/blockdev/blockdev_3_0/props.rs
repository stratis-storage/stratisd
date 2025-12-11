// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use tokio::sync::OwnedRwLockReadGuard;
use zbus::{fdo::Error, zvariant::OwnedObjectPath};

use crate::{
    dbus::{util::option_to_tuple, Manager},
    engine::{BlockDev, BlockDevTier, DevUuid, PoolUuid, SharedGuard},
};

pub fn devnode_prop(_: BlockDevTier, _: DevUuid, dev: &dyn BlockDev) -> String {
    dev.metadata_path().display().to_string()
}

pub fn hardware_info_prop(_: BlockDevTier, _: DevUuid, dev: &dyn BlockDev) -> (bool, String) {
    option_to_tuple(dev.hardware_info().map(|s| s.to_owned()), String::new())
}

pub fn init_time_prop(_: BlockDevTier, _: DevUuid, dev: &dyn BlockDev) -> Result<u64, Error> {
    u64::try_from(dev.initialization_time().timestamp()).map_err(|e| Error::Failed(e.to_string()))
}

pub fn physical_path_prop(_: BlockDevTier, _: DevUuid, dev: &dyn BlockDev) -> String {
    dev.devnode().display().to_string()
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

pub fn total_physical_size_prop(_: BlockDevTier, _: DevUuid, dev: &dyn BlockDev) -> String {
    (*dev.size().bytes()).to_string()
}

pub fn tier_prop(tier: BlockDevTier, _: DevUuid, _: &dyn BlockDev) -> u16 {
    tier as u16
}

pub fn user_info_prop(_: BlockDevTier, _: DevUuid, dev: &dyn BlockDev) -> (bool, String) {
    option_to_tuple(dev.user_info().map(|s| s.to_owned()), String::new())
}

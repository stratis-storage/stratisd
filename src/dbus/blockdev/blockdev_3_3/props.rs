// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use zbus::Error;

use crate::{
    dbus::util::{option_to_tuple, tuple_to_option},
    engine::{BlockDev, BlockDevTier, DevUuid, Pool, PoolUuid, SomeLockWriteGuard},
};

pub fn new_physical_size_prop(_: BlockDevTier, _: DevUuid, dev: &dyn BlockDev) -> (bool, String) {
    option_to_tuple(
        dev.new_size().map(|s| (*s.bytes()).to_string()),
        String::new(),
    )
}

pub fn set_user_info_prop(
    guard: &mut SomeLockWriteGuard<PoolUuid, dyn Pool>,
    dev_uuid: DevUuid,
    user_info_tuple: (bool, String),
) -> Result<(), Error> {
    let user_info = tuple_to_option(user_info_tuple);
    let user_info = user_info.as_deref();
    let (pool_name, _, pool) = guard.as_mut_tuple();
    if pool
        .get_blockdev(dev_uuid)
        .ok_or_else(|| Error::Failure(format!("Blockdev with UUID {dev_uuid} not found")))?
        .1
        .user_info()
        == user_info
    {
        Ok(())
    } else {
        pool.set_blockdev_user_info(&pool_name, dev_uuid, user_info)
            .map(|_| ())
            .map_err(|e| Error::Failure(e.to_string()))
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::arg::IterAppend;
use dbus_tree::{MTSync, MethodErr, PropInfo};

use crate::{
    dbus_api::{
        blockdev::shared::{self, get_blockdev_property},
        types::TData,
    },
    engine::Engine,
};

/// Get the new size for an object path representing a block device.
pub fn get_blockdev_new_size<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: Engine,
{
    get_blockdev_property(i, p, |_, p| Ok(shared::blockdev_new_size_prop::<E>(p)))
}

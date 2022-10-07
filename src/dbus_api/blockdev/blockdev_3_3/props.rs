// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::arg::{Iter, IterAppend};
use dbus_tree::{MTSync, MethodErr, PropInfo};

use crate::{
    dbus_api::{
        blockdev::shared::{
            self, get_blockdev_property, set_pool_level_blockdev_property_to_display,
        },
        consts,
        types::TData,
        util::tuple_to_option,
    },
    engine::{Engine, PropChangeAction},
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

pub fn get_blockdev_user_info<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: Engine,
{
    get_blockdev_property(i, p, |_, p| Ok(shared::blockdev_user_info_prop::<E>(p)))
}

pub fn set_blockdev_user_info<E>(
    i: &mut Iter<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: Engine,
{
    let user_info_tuple: (bool, String) = i
        .get()
        .ok_or_else(|| MethodErr::failed("User info required as argument to set it"))?;
    let user_info = tuple_to_option(user_info_tuple);
    let res = set_pool_level_blockdev_property_to_display(
        p,
        consts::BLOCKDEV_USER_INFO_PROP,
        |n, p, uuid| shared::set_blockdev_user_info_prop::<E>(p, n, uuid, user_info.as_deref()),
    );
    match res {
        Ok(PropChangeAction::NewValue(v)) => {
            p.tree
                .get_data()
                .push_blockdev_user_info_change(p.path.get_name(), v.map(|s| s.to_owned()));
            Ok(())
        }
        Ok(PropChangeAction::Identity) => Ok(()),
        Err(e) => Err(e),
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::arg::{Iter, IterAppend};
use dbus_tree::{MTSync, MethodErr, PropInfo};

use crate::{
    dbus_api::{
        consts,
        pool::shared::{self, get_pool_property, set_pool_property},
        types::TData,
    },
    engine::{Engine, PropChangeAction},
};

pub fn get_pool_fs_limit<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: 'static + Engine,
{
    get_pool_property(i, p, |(_, _, pool)| Ok(shared::pool_fs_limit::<E>(pool)))
}

pub fn set_pool_fs_limit<E>(
    i: &mut Iter<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: 'static + Engine,
{
    let fs_limit = i.get().ok_or_else(|| {
        MethodErr::failed("New filesystem limit required as argument to increase it")
    })?;
    let res = set_pool_property(p, consts::POOL_FS_LIMIT_PROP, |(name, uuid, pool)| {
        shared::set_pool_fs_limit::<E>(&name, uuid, pool, fs_limit)
    });
    match res {
        Ok(PropChangeAction::NewValue(v)) => {
            p.tree
                .get_data()
                .push_pool_fs_limit_change(p.path.get_name(), v);
            Ok(())
        }
        Ok(PropChangeAction::Identity) => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn get_overprov_mode<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: 'static + Engine,
{
    get_pool_property(i, p, |(_, _, pool)| {
        Ok(shared::pool_overprov_enabled::<E>(pool))
    })
}

pub fn set_overprov_mode<E>(
    i: &mut Iter<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: 'static + Engine,
{
    let disabled = i.get().ok_or_else(|| {
        MethodErr::failed("Overprovisioning mode changes require a boolean as an argument")
    })?;
    let res = set_pool_property(p, consts::POOL_OVERPROV_PROP, |(name, _, pool)| {
        shared::pool_set_overprov_mode::<E>(pool, &name, disabled)
    });
    match res {
        Ok(PropChangeAction::NewValue(v)) => {
            p.tree
                .get_data()
                .push_pool_overprov_mode_change(p.path.get_name(), v);
            Ok(())
        }
        Ok(PropChangeAction::Identity) => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn get_no_alloc_space<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: 'static + Engine,
{
    get_pool_property(i, p, |(_, _, pool)| {
        Ok(shared::pool_no_alloc_space::<E>(pool))
    })
}

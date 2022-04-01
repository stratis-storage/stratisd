// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::arg::{Iter, IterAppend};
use dbus_tree::{MTSync, MethodErr, PropInfo};

use crate::{
    dbus_api::{
        pool::shared::{self, get_pool_property, set_pool_property},
        types::TData,
    },
    engine::Engine,
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
    let res = set_pool_property(p, |(name, uuid, pool)| {
        shared::set_pool_fs_limit::<E>(&name, uuid, pool, fs_limit)
    });
    if res.is_ok() {
        p.tree
            .get_data()
            .push_pool_fs_limit_change(p.path.get_name(), fs_limit);
    }
    res
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
    let res = set_pool_property(p, |(name, _, pool)| {
        shared::pool_set_overprov_mode::<E>(pool, &name, disabled)
    });
    if res.is_ok() {
        p.tree
            .get_data()
            .push_pool_overprov_mode_change(p.path.get_name(), disabled);
    }
    res
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

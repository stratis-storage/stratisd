// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::arg::IterAppend;
use dbus_tree::{MTSync, MethodErr, PropInfo};

use crate::{
    dbus_api::{
        pool::shared::{self, get_pool_property},
        types::TData,
    },
    engine::Engine,
};

pub fn get_pool_name<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: 'static + Engine,
{
    get_pool_property(i, p, |(name, _, _)| Ok(shared::pool_name_prop(&name)))
}

pub fn get_pool_encrypted<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: 'static + Engine,
{
    get_pool_property(i, p, |(_, _, pool)| Ok(shared::pool_enc_prop::<E>(pool)))
}

pub fn get_pool_avail_actions<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: 'static + Engine,
{
    get_pool_property(i, p, |(_, _, pool)| {
        Ok(shared::pool_avail_actions_prop::<E>(pool))
    })
}

pub fn get_pool_key_desc<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: 'static + Engine,
{
    get_pool_property(i, p, |(_, _, pool)| {
        Ok(shared::pool_key_desc_prop::<E>(pool))
    })
}

pub fn get_pool_clevis_info<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: 'static + Engine,
{
    get_pool_property(i, p, |(_, _, pool)| {
        Ok(shared::pool_clevis_info_prop::<E>(pool))
    })
}

pub fn get_pool_has_cache<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: 'static + Engine,
{
    get_pool_property(i, p, |(_, _, pool)| {
        Ok(shared::pool_has_cache_prop::<E>(pool))
    })
}

pub fn get_pool_used_size<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: 'static + Engine,
{
    get_pool_property(i, p, |(_, _, pool)| Ok(shared::pool_used_size::<E>(pool)))
}

pub fn get_pool_allocated_size<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: 'static + Engine,
{
    get_pool_property(i, p, |(_, _, pool)| {
        Ok(shared::pool_allocated_size::<E>(pool))
    })
}

pub fn get_pool_total_size<E>(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: 'static + Engine,
{
    get_pool_property(i, p, |(_, _, pool)| Ok(shared::pool_total_size::<E>(pool)))
}

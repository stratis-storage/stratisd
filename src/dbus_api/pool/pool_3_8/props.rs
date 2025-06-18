// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::arg::IterAppend;
use dbus_tree::{MTSync, MethodErr, PropInfo};

use crate::dbus_api::{
    pool::shared::{self, get_pool_property},
    types::TData,
};

pub fn get_pool_metadata_version(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    get_pool_property(i, p, |(_, _, pool)| Ok(shared::pool_metadata_version(pool)))
}

pub fn get_pool_key_descs(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    get_pool_property(i, p, |(_, _, pool)| Ok(shared::pool_key_descs_prop(pool)))
}

pub fn get_pool_clevis_infos(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    get_pool_property(i, p, |(_, _, pool)| {
        Ok(shared::pool_clevis_infos_prop(pool))
    })
}

pub fn get_pool_free_token_slots(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    get_pool_property(i, p, |(_, _, pool)| Ok(shared::pool_free_token_slots(pool)))
}

pub fn get_pool_volume_key_loaded(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    get_pool_property(i, p, |(_, uuid, pool)| {
        Ok(shared::pool_volume_key_loaded(pool, uuid))
    })
}

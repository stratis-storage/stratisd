// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::arg::IterAppend;
use dbus_tree::{MTSync, MethodErr, PropInfo};

use crate::{
    dbus_api::{
        api::shared::{self, get_manager_property},
        types::TData,
    },
    stratis::VERSION,
};

pub fn get_version(
    i: &mut IterAppend<'_>,
    _: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    i.append(VERSION);
    Ok(())
}

pub fn get_locked_pools(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    get_manager_property(i, p, |e| Ok(shared::locked_pools_prop(e)))
}

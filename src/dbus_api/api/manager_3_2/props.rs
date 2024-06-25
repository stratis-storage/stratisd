// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::arg::IterAppend;
use dbus_tree::{MTSync, MethodErr, PropInfo};

use crate::dbus_api::{
    api::shared::{self, get_manager_property},
    types::TData,
};

pub fn get_stopped_pools(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    get_manager_property(i, p, |e| Ok(shared::stopped_pools_prop(e, false)))
}

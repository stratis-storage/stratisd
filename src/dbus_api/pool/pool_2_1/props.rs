// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{
    self,
    arg::IterAppend,
    tree::{MTFn, MethodErr, PropInfo},
};

use crate::dbus_api::{pool::shared::get_pool_property, types::TData};

pub fn get_pool_encrypted(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_pool_property(i, p, |(_, _, pool)| Ok(pool.is_encrypted()))
}

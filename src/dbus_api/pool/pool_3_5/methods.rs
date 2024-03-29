// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{MTSync, MethodInfo, MethodResult};

use crate::dbus_api::{
    pool::shared::{add_blockdevs, BlockDevOp},
    types::TData,
};

pub fn init_cache(m: &MethodInfo<'_, MTSync<TData>, TData>) -> MethodResult {
    add_blockdevs(m, BlockDevOp::InitCacheWithEnc)
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{MTSync, MethodInfo, MethodResult};

use crate::{
    dbus_api::{
        pool::shared::{add_blockdevs, BlockDevOp},
        types::TData,
    },
    engine::Engine,
};

pub fn init_cache<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> MethodResult
where
    E: 'static + Engine,
{
    add_blockdevs(m, BlockDevOp::InitCacheWithEnc)
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::tree::{MTFn, MethodInfo, MethodResult};

use crate::dbus_api::{api::shared::create_pool_shared, types::TData};

pub fn create_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    create_pool_shared(m, true)
}

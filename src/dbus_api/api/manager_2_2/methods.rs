// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::tree::{MTSync, MethodInfo, MethodResult};

use crate::dbus_api::{api::shared::set_key_shared, types::TData};

pub fn set_key(m: &MethodInfo<MTSync<TData>, TData>) -> MethodResult {
    set_key_shared(m)
}

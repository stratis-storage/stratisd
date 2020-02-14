// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::tree::{Access, EmitsChangedSignal, Factory, MTFn, Property};

use crate::dbus_api::{consts, pool::pool_2_1::props::get_pool_encrypted, types::TData};

pub fn encrypted_property(f: &Factory<MTFn<TData>, TData>) -> Property<MTFn<TData>, TData> {
    f.property::<bool, _>(consts::POOL_ENCRYPTED_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_pool_encrypted)
}

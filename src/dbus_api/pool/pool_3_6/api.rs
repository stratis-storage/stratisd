// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Access, EmitsChangedSignal, Factory, MTSync, Property};

use crate::dbus_api::{consts, pool::pool_3_6::props::get_pool_metadata_version, types::TData};

pub fn metadata_version_property(
    f: &Factory<MTSync<TData>, TData>,
) -> Property<MTSync<TData>, TData> {
    f.property::<u64, _>(consts::POOL_METADATA_VERSION_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_pool_metadata_version)
}

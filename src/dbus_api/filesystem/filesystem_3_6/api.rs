// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Access, EmitsChangedSignal, Factory, MTSync, Property};

use crate::dbus_api::{
    consts,
    filesystem::filesystem_3_6::props::{get_fs_size_limit, set_fs_size_limit},
    types::TData,
};

pub fn size_limit_property(f: &Factory<MTSync<TData>, TData>) -> Property<MTSync<TData>, TData> {
    f.property::<(bool, String), _>(consts::FILESYSTEM_SIZE_LIMIT_PROP, ())
        .access(Access::ReadWrite)
        .emits_changed(EmitsChangedSignal::True)
        .auto_emit_on_set(false)
        .on_get(get_fs_size_limit)
        .on_set(set_fs_size_limit)
}

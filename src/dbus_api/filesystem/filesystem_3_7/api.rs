// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Access, EmitsChangedSignal, Factory, MTSync, Property};

use crate::dbus_api::{
    consts,
    filesystem::filesystem_3_7::props::{
        get_fs_merge_scheduled, get_fs_origin, set_fs_merge_scheduled,
    },
    types::TData,
};

pub fn origin_property(f: &Factory<MTSync<TData>, TData>) -> Property<MTSync<TData>, TData> {
    f.property::<(bool, String), _>(consts::FILESYSTEM_ORIGIN_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_fs_origin)
}

pub fn merge_scheduled_property(
    f: &Factory<MTSync<TData>, TData>,
) -> Property<MTSync<TData>, TData> {
    f.property::<bool, _>(consts::FILESYSTEM_MERGE_SCHEDULED_PROP, ())
        .access(Access::ReadWrite)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_fs_merge_scheduled)
        .on_set(set_fs_merge_scheduled)
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::tree::{Access, EmitsChangedSignal, Factory, MTSync, Property};

use crate::dbus_api::{
    blockdev::blockdev_2_2::props::get_blockdev_physical_path, consts, types::TData,
};

pub fn physical_path_property(f: &Factory<MTSync<TData>, TData>) -> Property<MTSync<TData>, TData> {
    f.property::<&str, _>(consts::BLOCKDEV_PHYSICAL_PATH_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_blockdev_physical_path)
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Access, EmitsChangedSignal, Factory, MTSync, Property};

use crate::{
    dbus_api::{
        blockdev::blockdev_3_3::props::{
            get_blockdev_new_size, get_blockdev_user_info, set_blockdev_user_info,
        },
        consts,
        types::TData,
    },
    engine::Engine,
};

pub fn new_size_property<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Property<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.property::<(bool, &str), _>(consts::BLOCKDEV_NEW_SIZE_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_blockdev_new_size)
}

pub fn user_info_property<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Property<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.property::<(bool, &str), _>(consts::BLOCKDEV_USER_INFO_PROP, ())
        .access(Access::ReadWrite)
        .emits_changed(EmitsChangedSignal::True)
        .auto_emit_on_set(false)
        .on_get(get_blockdev_user_info)
        .on_set(set_blockdev_user_info)
}

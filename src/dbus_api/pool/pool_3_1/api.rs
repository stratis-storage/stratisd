// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Access, EmitsChangedSignal, Factory, MTSync, Property};

use crate::{
    dbus_api::{
        consts,
        pool::pool_3_1::props::{
            get_no_alloc_space, get_overprov_mode, get_pool_fs_limit, set_overprov_mode,
            set_pool_fs_limit,
        },
        types::TData,
    },
    engine::Engine,
};

pub fn fs_limit_property<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Property<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.property::<u64, _>(consts::POOL_FS_LIMIT_PROP, ())
        .access(Access::ReadWrite)
        .emits_changed(EmitsChangedSignal::True)
        .auto_emit_on_set(false)
        .on_get(get_pool_fs_limit)
        .on_set(set_pool_fs_limit)
}

pub fn enable_overprov_property<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Property<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.property::<bool, _>(consts::POOL_OVERPROV_PROP, ())
        .access(Access::ReadWrite)
        .emits_changed(EmitsChangedSignal::True)
        .auto_emit_on_set(false)
        .on_get(get_overprov_mode)
        .on_set(set_overprov_mode)
}

pub fn no_alloc_space_property<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Property<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.property::<bool, _>(consts::POOL_NO_ALLOCABLE_SPACE_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_no_alloc_space)
}

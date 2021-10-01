// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Access, EmitsChangedSignal, Factory, MTSync, Method, Property};

use crate::{
    dbus_api::{
        blockdev::blockdev_3_0::{
            methods::set_user_info,
            props::{
                get_blockdev_devnode, get_blockdev_hardware_info, get_blockdev_initialization_time,
                get_blockdev_physical_path, get_blockdev_tier, get_blockdev_user_info,
            },
        },
        consts,
        types::TData,
        util::{get_parent, get_uuid},
    },
    engine::Engine,
};

pub fn set_userid_method<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Method<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.method("SetUserInfo", (), set_user_info)
        .in_arg(("id", "(bs)"))
        // b: false if no change to the user info
        // s: UUID of the changed device
        //
        // Rust representation: (bool, String)
        .out_arg(("changed", "(bs)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn devnode_property<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Property<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.property::<&str, _>(consts::BLOCKDEV_DEVNODE_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_blockdev_devnode)
}

pub fn hardware_info_property<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Property<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.property::<(bool, &str), _>(consts::BLOCKDEV_HARDWARE_INFO_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_blockdev_hardware_info)
}

pub fn user_info_property<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Property<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.property::<(bool, &str), _>(consts::BLOCKDEV_USER_INFO_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::False)
        .on_get(get_blockdev_user_info)
}

pub fn initialization_time_property<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Property<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.property::<u64, _>(consts::BLOCKDEV_INIT_TIME_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_blockdev_initialization_time)
}

pub fn pool_property<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Property<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.property::<&dbus::Path<'_>, _>(consts::BLOCKDEV_POOL_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_parent)
}

pub fn uuid_property<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Property<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.property::<&str, _>(consts::BLOCKDEV_UUID_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_uuid)
}

pub fn tier_property<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Property<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.property::<u16, _>(consts::BLOCKDEV_TIER_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::False)
        .on_get(get_blockdev_tier)
}

pub fn physical_path_property<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Property<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.property::<&str, _>(consts::BLOCKDEV_PHYSICAL_PATH_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_blockdev_physical_path)
}

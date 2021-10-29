// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Access, EmitsChangedSignal, Factory, MTSync, Method, Property};

use crate::{
    dbus_api::{
        api::{
            manager_3_0::{
                methods::{
                    create_pool, destroy_pool, engine_state_report, list_keys, set_key,
                    unlock_pool, unset_key,
                },
                props::{get_locked_pools, get_version},
            },
            prop_conv::LockedPools,
        },
        consts,
        types::TData,
    },
    engine::Engine,
};

pub fn destroy_pool_method<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Method<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.method("DestroyPool", (), destroy_pool)
        .in_arg(("pool", "o"))
        // In order from left to right:
        // b: true if a valid UUID is returned - otherwise no action was performed
        // s: String representation of pool UUID that was destroyed
        //
        // Rust representation: (bool, String)
        .out_arg(("result", "(bs)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn list_keys_method<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Method<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.method("ListKeys", (), list_keys)
        // In order from left to right:
        // as: Array of key descriptions as strings.
        //
        // Rust representation: Vec<String>
        .out_arg(("result", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn version_property<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Property<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.property::<&str, _>("Version", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_version::<E>)
}

pub fn unset_key_method<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Method<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.method("UnsetKey", (), unset_key)
        .in_arg(("key_desc", "s"))
        // b: true if the key was unset from the keyring. false if the key
        //    was not present in the keyring before the operation.
        //
        // Rust representation: bool
        .out_arg(("result", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn set_key_method<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Method<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.method("SetKey", (), set_key)
        .in_arg(("key_desc", "s"))
        .in_arg(("key_fd", "h"))
        // b: true if the key state was changed in the kernel keyring.
        // b: true if the key description already existed in the kernel keyring and
        //    the key data has been changed to a new value.
        //
        // Rust representation: (bool, bool)
        .out_arg(("result", "(bb)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn unlock_pool_method<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Method<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.method("UnlockPool", (), unlock_pool)
        .in_arg(("pool_uuid", "s"))
        .in_arg(("unlock_method", "s"))
        // b: true if some encrypted devices were newly opened.
        // as: array of device UUIDs converted to Strings of all of the newly opened
        //     devices.
        //
        // Rust representation: (bool, Vec<DevUuid>)
        .out_arg(("result", "(bas)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn engine_state_report_method<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Method<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.method("EngineStateReport", (), engine_state_report)
        // s: JSON engine state report as a string.
        //
        // Rust representation: Value
        .out_arg(("result", "s"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn create_pool_method<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Method<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.method("CreatePool", (), create_pool)
        .in_arg(("name", "s"))
        .in_arg(("redundancy", "(bq)"))
        .in_arg(("devices", "as"))
        // Optional key description of key in the kernel keyring
        // b: true if the pool should be encrypted and able to be
        // unlocked with a passphrase associated with this key description.
        // s: key description
        //
        // Rust representation: (bool, String)
        .in_arg(("key_desc", "(bs)"))
        // Optional Clevis information for binding on initialization.
        // b: true if the pool should be encrypted and able to be unlocked
        // using Clevis.
        // s: pin name
        // s: JSON config for Clevis use
        //
        // Rust representation: (bool, (String, String))
        .in_arg(("clevis_info", "(b(ss))"))
        // In order from left to right:
        // b: true if a pool was created and object paths were returned
        // o: Object path for Pool
        // a(o): Array of object paths for block devices
        //
        // Rust representation: (bool, (dbus::Path, Vec<dbus::Path>))
        .out_arg(("result", "(b(oao))"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn locked_pools_property<E>(
    f: &Factory<MTSync<TData<E>>, TData<E>>,
) -> Property<MTSync<TData<E>>, TData<E>>
where
    E: 'static + Engine,
{
    f.property::<LockedPools, _>(consts::LOCKED_POOLS_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_locked_pools::<E>)
}

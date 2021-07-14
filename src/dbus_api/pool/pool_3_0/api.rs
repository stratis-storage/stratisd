// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::{Access, EmitsChangedSignal, Factory, MTSync, Method, Property};

use crate::dbus_api::{
    consts,
    pool::pool_3_0::{
        methods::{
            add_cachedevs, add_datadevs, bind_clevis, bind_keyring, create_filesystems,
            destroy_filesystems, init_cache, rebind_clevis, rebind_keyring, rename_pool,
            snapshot_filesystem, unbind_clevis, unbind_keyring,
        },
        props::{get_pool_encrypted, get_pool_name},
    },
    types::TData,
    util::get_uuid,
};

pub fn create_filesystems_method(
    f: &Factory<MTSync<TData>, TData>,
) -> Method<MTSync<TData>, TData> {
    f.method("CreateFilesystems", (), create_filesystems)
        .in_arg(("specs", "a(s(bs))"))
        // b: true if filesystems were created
        // a(os): Array of tuples with object paths and names
        //
        // Rust representation: (bool, Vec<(dbus::Path, String)>)
        .out_arg(("results", "(ba(os))"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn destroy_filesystems_method(
    f: &Factory<MTSync<TData>, TData>,
) -> Method<MTSync<TData>, TData> {
    f.method("DestroyFilesystems", (), destroy_filesystems)
        .in_arg(("filesystems", "ao"))
        // b: true if filesystems were destroyed
        // as: Array of UUIDs of destroyed filesystems
        //
        // Rust representation: (bool, Vec<String>)
        .out_arg(("results", "(bas)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn snapshot_filesystem_method(
    f: &Factory<MTSync<TData>, TData>,
) -> Method<MTSync<TData>, TData> {
    f.method("SnapshotFilesystem", (), snapshot_filesystem)
        .in_arg(("origin", "o"))
        .in_arg(("snapshot_name", "s"))
        // b: false if no new snapshot was created
        // s: Object path of new snapshot
        //
        // Rust representation: (bool, String)
        .out_arg(("result", "(bo)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn add_blockdevs_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("AddDataDevs", (), add_datadevs)
        .in_arg(("devices", "as"))
        // b: Indicates if any data devices were added
        // ao: Array of object paths of created data devices
        //
        // Rust representation: (bool, Vec<dbus::path>)
        .out_arg(("results", "(bao)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn rename_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("SetName", (), rename_pool)
        .in_arg(("name", "s"))
        // b: false if no pool was renamed
        // s: UUID of renamed pool
        //
        // Rust representation: (bool, String)
        .out_arg(("result", "(bs)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn name_property(f: &Factory<MTSync<TData>, TData>) -> Property<MTSync<TData>, TData> {
    f.property::<&str, _>(consts::POOL_NAME_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_pool_name)
}

pub fn uuid_property(f: &Factory<MTSync<TData>, TData>) -> Property<MTSync<TData>, TData> {
    f.property::<&str, _>(consts::POOL_UUID_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_uuid)
}

pub fn init_cache_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("InitCache", (), init_cache)
        .in_arg(("devices", "as"))
        // b: Indicates if any cache devices were added
        // ao: Array of object paths of created cache devices
        //
        // Rust representation: (bool, Vec<dbus::path>)
        .out_arg(("results", "(bao)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn add_cachedevs_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("AddCacheDevs", (), add_cachedevs)
        .in_arg(("devices", "as"))
        // b: Indicates if any cache devices were added
        // ao: Array of object paths of created cache devices
        //
        // Rust representation: (bool, Vec<dbus::path>)
        .out_arg(("results", "(bao)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn encrypted_property(f: &Factory<MTSync<TData>, TData>) -> Property<MTSync<TData>, TData> {
    f.property::<bool, _>(consts::POOL_ENCRYPTED_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_pool_encrypted)
}

pub fn bind_clevis_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("Bind", (), bind_clevis)
        .in_arg(("pin", "s"))
        .in_arg(("json", "s"))
        // b: Indicates if new clevis bindings were added
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn unbind_clevis_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("Unbind", (), unbind_clevis)
        // b: Indicates if clevis bindings were removed
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn bind_keyring_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("BindKeyring", (), bind_keyring)
        .in_arg(("key_desc", "s"))
        // b: Indicates if new keyring bindings were added
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn unbind_keyring_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("UnbindKeyring", (), unbind_keyring)
        // b: Indicates if keyring bindings were removed
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn rebind_keyring_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("RebindKeyring", (), rebind_keyring)
        .in_arg(("key_desc", "s"))
        // b: Indicates if keyring bindings were changed
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

pub fn rebind_clevis_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("RebindClevis", (), rebind_clevis)
        // b: Indicates if Clevis bindings were changed
        //
        // Rust representation: bool
        .out_arg(("results", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}

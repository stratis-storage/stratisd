// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::Factory;

use crate::{
    dbus_api::{
        consts,
        types::{DbusContext, InterfacesAddedThreadSafe, OPContext},
        util::make_object_path,
    },
    engine::{Name, Pool, PoolUuid, StratisUuid},
};

mod pool_3_0;
mod pool_3_1;
mod pool_3_3;
mod pool_3_5;
mod pool_3_6;
mod pool_3_7;
mod pool_3_8;
pub mod prop_conv;
mod shared;

pub fn create_dbus_pool<'a>(
    dbus_context: &DbusContext,
    parent: dbus::Path<'static>,
    name: &Name,
    uuid: PoolUuid,
    pool: &dyn Pool,
) -> dbus::Path<'a> {
    let f = Factory::new_sync();

    let object_name = make_object_path(dbus_context);

    let object_path = f
        .object_path(
            object_name,
            Some(OPContext::new(parent, StratisUuid::Pool(uuid))),
        )
        .introspectable()
        .add(
            f.interface(consts::POOL_INTERFACE_NAME_3_0, ())
                .add_m(pool_3_0::create_filesystems_method(&f))
                .add_m(pool_3_0::destroy_filesystems_method(&f))
                .add_m(pool_3_0::snapshot_filesystem_method(&f))
                .add_m(pool_3_0::add_blockdevs_method(&f))
                .add_m(pool_3_0::bind_clevis_method(&f))
                .add_m(pool_3_0::unbind_clevis_method(&f))
                .add_m(pool_3_0::init_cache_method(&f))
                .add_m(pool_3_0::add_cachedevs_method(&f))
                .add_m(pool_3_0::bind_keyring_method(&f))
                .add_m(pool_3_0::unbind_keyring_method(&f))
                .add_m(pool_3_0::rebind_keyring_method(&f))
                .add_m(pool_3_0::rebind_clevis_method(&f))
                .add_m(pool_3_0::rename_method(&f))
                .add_p(pool_3_0::name_property(&f))
                .add_p(pool_3_0::uuid_property(&f))
                .add_p(pool_3_0::encrypted_property(&f))
                .add_p(pool_3_0::avail_actions_property(&f))
                .add_p(pool_3_0::key_desc_property(&f))
                .add_p(pool_3_0::clevis_info_property(&f))
                .add_p(pool_3_0::has_cache_property(&f))
                .add_p(pool_3_0::alloc_size_property(&f))
                .add_p(pool_3_0::used_size_property(&f))
                .add_p(pool_3_0::total_size_property(&f)),
        )
        .add(
            f.interface(consts::POOL_INTERFACE_NAME_3_1, ())
                .add_m(pool_3_0::create_filesystems_method(&f))
                .add_m(pool_3_0::destroy_filesystems_method(&f))
                .add_m(pool_3_0::snapshot_filesystem_method(&f))
                .add_m(pool_3_0::add_blockdevs_method(&f))
                .add_m(pool_3_0::bind_clevis_method(&f))
                .add_m(pool_3_0::unbind_clevis_method(&f))
                .add_m(pool_3_0::init_cache_method(&f))
                .add_m(pool_3_0::add_cachedevs_method(&f))
                .add_m(pool_3_0::bind_keyring_method(&f))
                .add_m(pool_3_0::unbind_keyring_method(&f))
                .add_m(pool_3_0::rebind_keyring_method(&f))
                .add_m(pool_3_0::rebind_clevis_method(&f))
                .add_m(pool_3_0::rename_method(&f))
                .add_p(pool_3_0::name_property(&f))
                .add_p(pool_3_0::uuid_property(&f))
                .add_p(pool_3_0::encrypted_property(&f))
                .add_p(pool_3_0::avail_actions_property(&f))
                .add_p(pool_3_0::key_desc_property(&f))
                .add_p(pool_3_0::clevis_info_property(&f))
                .add_p(pool_3_0::has_cache_property(&f))
                .add_p(pool_3_0::alloc_size_property(&f))
                .add_p(pool_3_0::used_size_property(&f))
                .add_p(pool_3_0::total_size_property(&f))
                .add_p(pool_3_1::fs_limit_property(&f))
                .add_p(pool_3_1::enable_overprov_property(&f))
                .add_p(pool_3_1::no_alloc_space_property(&f)),
        )
        .add(
            f.interface(consts::POOL_INTERFACE_NAME_3_2, ())
                .add_m(pool_3_0::create_filesystems_method(&f))
                .add_m(pool_3_0::destroy_filesystems_method(&f))
                .add_m(pool_3_0::snapshot_filesystem_method(&f))
                .add_m(pool_3_0::add_blockdevs_method(&f))
                .add_m(pool_3_0::bind_clevis_method(&f))
                .add_m(pool_3_0::unbind_clevis_method(&f))
                .add_m(pool_3_0::init_cache_method(&f))
                .add_m(pool_3_0::add_cachedevs_method(&f))
                .add_m(pool_3_0::bind_keyring_method(&f))
                .add_m(pool_3_0::unbind_keyring_method(&f))
                .add_m(pool_3_0::rebind_keyring_method(&f))
                .add_m(pool_3_0::rebind_clevis_method(&f))
                .add_m(pool_3_0::rename_method(&f))
                .add_p(pool_3_0::name_property(&f))
                .add_p(pool_3_0::uuid_property(&f))
                .add_p(pool_3_0::encrypted_property(&f))
                .add_p(pool_3_0::avail_actions_property(&f))
                .add_p(pool_3_0::key_desc_property(&f))
                .add_p(pool_3_0::clevis_info_property(&f))
                .add_p(pool_3_0::has_cache_property(&f))
                .add_p(pool_3_0::alloc_size_property(&f))
                .add_p(pool_3_0::used_size_property(&f))
                .add_p(pool_3_0::total_size_property(&f))
                .add_p(pool_3_1::fs_limit_property(&f))
                .add_p(pool_3_1::enable_overprov_property(&f))
                .add_p(pool_3_1::no_alloc_space_property(&f)),
        )
        .add(
            f.interface(consts::POOL_INTERFACE_NAME_3_3, ())
                .add_m(pool_3_0::create_filesystems_method(&f))
                .add_m(pool_3_0::destroy_filesystems_method(&f))
                .add_m(pool_3_0::snapshot_filesystem_method(&f))
                .add_m(pool_3_0::add_blockdevs_method(&f))
                .add_m(pool_3_0::bind_clevis_method(&f))
                .add_m(pool_3_0::unbind_clevis_method(&f))
                .add_m(pool_3_0::init_cache_method(&f))
                .add_m(pool_3_0::add_cachedevs_method(&f))
                .add_m(pool_3_0::bind_keyring_method(&f))
                .add_m(pool_3_0::unbind_keyring_method(&f))
                .add_m(pool_3_0::rebind_keyring_method(&f))
                .add_m(pool_3_0::rebind_clevis_method(&f))
                .add_m(pool_3_0::rename_method(&f))
                .add_m(pool_3_3::grow_physical_device_method(&f))
                .add_p(pool_3_0::name_property(&f))
                .add_p(pool_3_0::uuid_property(&f))
                .add_p(pool_3_0::encrypted_property(&f))
                .add_p(pool_3_0::avail_actions_property(&f))
                .add_p(pool_3_0::key_desc_property(&f))
                .add_p(pool_3_0::clevis_info_property(&f))
                .add_p(pool_3_0::has_cache_property(&f))
                .add_p(pool_3_0::alloc_size_property(&f))
                .add_p(pool_3_0::used_size_property(&f))
                .add_p(pool_3_0::total_size_property(&f))
                .add_p(pool_3_1::fs_limit_property(&f))
                .add_p(pool_3_1::enable_overprov_property(&f))
                .add_p(pool_3_1::no_alloc_space_property(&f)),
        )
        .add(
            f.interface(consts::POOL_INTERFACE_NAME_3_4, ())
                .add_m(pool_3_0::create_filesystems_method(&f))
                .add_m(pool_3_0::destroy_filesystems_method(&f))
                .add_m(pool_3_0::snapshot_filesystem_method(&f))
                .add_m(pool_3_0::add_blockdevs_method(&f))
                .add_m(pool_3_0::bind_clevis_method(&f))
                .add_m(pool_3_0::unbind_clevis_method(&f))
                .add_m(pool_3_0::init_cache_method(&f))
                .add_m(pool_3_0::add_cachedevs_method(&f))
                .add_m(pool_3_0::bind_keyring_method(&f))
                .add_m(pool_3_0::unbind_keyring_method(&f))
                .add_m(pool_3_0::rebind_keyring_method(&f))
                .add_m(pool_3_0::rebind_clevis_method(&f))
                .add_m(pool_3_0::rename_method(&f))
                .add_m(pool_3_3::grow_physical_device_method(&f))
                .add_p(pool_3_0::name_property(&f))
                .add_p(pool_3_0::uuid_property(&f))
                .add_p(pool_3_0::encrypted_property(&f))
                .add_p(pool_3_0::avail_actions_property(&f))
                .add_p(pool_3_0::key_desc_property(&f))
                .add_p(pool_3_0::clevis_info_property(&f))
                .add_p(pool_3_0::has_cache_property(&f))
                .add_p(pool_3_0::alloc_size_property(&f))
                .add_p(pool_3_0::used_size_property(&f))
                .add_p(pool_3_0::total_size_property(&f))
                .add_p(pool_3_1::fs_limit_property(&f))
                .add_p(pool_3_1::enable_overprov_property(&f))
                .add_p(pool_3_1::no_alloc_space_property(&f)),
        )
        .add(
            f.interface(consts::POOL_INTERFACE_NAME_3_5, ())
                .add_m(pool_3_0::create_filesystems_method(&f))
                .add_m(pool_3_0::destroy_filesystems_method(&f))
                .add_m(pool_3_0::snapshot_filesystem_method(&f))
                .add_m(pool_3_0::add_blockdevs_method(&f))
                .add_m(pool_3_0::bind_clevis_method(&f))
                .add_m(pool_3_0::unbind_clevis_method(&f))
                .add_m(pool_3_5::init_cache_method(&f))
                .add_m(pool_3_0::add_cachedevs_method(&f))
                .add_m(pool_3_0::bind_keyring_method(&f))
                .add_m(pool_3_0::unbind_keyring_method(&f))
                .add_m(pool_3_0::rebind_keyring_method(&f))
                .add_m(pool_3_0::rebind_clevis_method(&f))
                .add_m(pool_3_0::rename_method(&f))
                .add_m(pool_3_3::grow_physical_device_method(&f))
                .add_p(pool_3_0::name_property(&f))
                .add_p(pool_3_0::uuid_property(&f))
                .add_p(pool_3_0::encrypted_property(&f))
                .add_p(pool_3_0::avail_actions_property(&f))
                .add_p(pool_3_0::key_desc_property(&f))
                .add_p(pool_3_0::clevis_info_property(&f))
                .add_p(pool_3_0::has_cache_property(&f))
                .add_p(pool_3_0::alloc_size_property(&f))
                .add_p(pool_3_0::used_size_property(&f))
                .add_p(pool_3_0::total_size_property(&f))
                .add_p(pool_3_1::fs_limit_property(&f))
                .add_p(pool_3_1::enable_overprov_property(&f))
                .add_p(pool_3_1::no_alloc_space_property(&f)),
        )
        .add(
            f.interface(consts::POOL_INTERFACE_NAME_3_6, ())
                .add_m(pool_3_6::create_filesystems_method(&f))
                .add_m(pool_3_0::destroy_filesystems_method(&f))
                .add_m(pool_3_0::snapshot_filesystem_method(&f))
                .add_m(pool_3_0::add_blockdevs_method(&f))
                .add_m(pool_3_0::bind_clevis_method(&f))
                .add_m(pool_3_0::unbind_clevis_method(&f))
                .add_m(pool_3_5::init_cache_method(&f))
                .add_m(pool_3_0::add_cachedevs_method(&f))
                .add_m(pool_3_0::bind_keyring_method(&f))
                .add_m(pool_3_0::unbind_keyring_method(&f))
                .add_m(pool_3_0::rebind_keyring_method(&f))
                .add_m(pool_3_0::rebind_clevis_method(&f))
                .add_m(pool_3_0::rename_method(&f))
                .add_m(pool_3_3::grow_physical_device_method(&f))
                .add_p(pool_3_0::name_property(&f))
                .add_p(pool_3_0::uuid_property(&f))
                .add_p(pool_3_0::encrypted_property(&f))
                .add_p(pool_3_0::avail_actions_property(&f))
                .add_p(pool_3_0::key_desc_property(&f))
                .add_p(pool_3_0::clevis_info_property(&f))
                .add_p(pool_3_0::has_cache_property(&f))
                .add_p(pool_3_0::alloc_size_property(&f))
                .add_p(pool_3_0::used_size_property(&f))
                .add_p(pool_3_0::total_size_property(&f))
                .add_p(pool_3_1::fs_limit_property(&f))
                .add_p(pool_3_1::enable_overprov_property(&f))
                .add_p(pool_3_1::no_alloc_space_property(&f)),
        )
        .add(
            f.interface(consts::POOL_INTERFACE_NAME_3_7, ())
                .add_m(pool_3_6::create_filesystems_method(&f))
                .add_m(pool_3_7::destroy_filesystems_method(&f))
                .add_m(pool_3_0::snapshot_filesystem_method(&f))
                .add_m(pool_3_0::add_blockdevs_method(&f))
                .add_m(pool_3_0::bind_clevis_method(&f))
                .add_m(pool_3_0::unbind_clevis_method(&f))
                .add_m(pool_3_5::init_cache_method(&f))
                .add_m(pool_3_0::add_cachedevs_method(&f))
                .add_m(pool_3_0::bind_keyring_method(&f))
                .add_m(pool_3_0::unbind_keyring_method(&f))
                .add_m(pool_3_0::rebind_keyring_method(&f))
                .add_m(pool_3_0::rebind_clevis_method(&f))
                .add_m(pool_3_0::rename_method(&f))
                .add_m(pool_3_3::grow_physical_device_method(&f))
                .add_m(pool_3_7::get_metadata_method(&f))
                .add_m(pool_3_7::get_fs_metadata_method(&f))
                .add_p(pool_3_0::name_property(&f))
                .add_p(pool_3_0::uuid_property(&f))
                .add_p(pool_3_0::encrypted_property(&f))
                .add_p(pool_3_0::avail_actions_property(&f))
                .add_p(pool_3_0::key_desc_property(&f))
                .add_p(pool_3_0::clevis_info_property(&f))
                .add_p(pool_3_0::has_cache_property(&f))
                .add_p(pool_3_0::alloc_size_property(&f))
                .add_p(pool_3_0::used_size_property(&f))
                .add_p(pool_3_0::total_size_property(&f))
                .add_p(pool_3_1::fs_limit_property(&f))
                .add_p(pool_3_1::enable_overprov_property(&f))
                .add_p(pool_3_1::no_alloc_space_property(&f)),
        )
        .add(
            f.interface(consts::POOL_INTERFACE_NAME_3_8, ())
                .add_m(pool_3_6::create_filesystems_method(&f))
                .add_m(pool_3_7::destroy_filesystems_method(&f))
                .add_m(pool_3_0::snapshot_filesystem_method(&f))
                .add_m(pool_3_0::add_blockdevs_method(&f))
                .add_m(pool_3_8::bind_clevis_method(&f))
                .add_m(pool_3_8::unbind_clevis_method(&f))
                .add_m(pool_3_5::init_cache_method(&f))
                .add_m(pool_3_0::add_cachedevs_method(&f))
                .add_m(pool_3_8::bind_keyring_method(&f))
                .add_m(pool_3_8::unbind_keyring_method(&f))
                .add_m(pool_3_8::rebind_keyring_method(&f))
                .add_m(pool_3_8::rebind_clevis_method(&f))
                .add_m(pool_3_0::rename_method(&f))
                .add_m(pool_3_3::grow_physical_device_method(&f))
                .add_m(pool_3_7::get_metadata_method(&f))
                .add_m(pool_3_7::get_fs_metadata_method(&f))
                .add_m(pool_3_8::encrypt_pool_method(&f))
                .add_p(pool_3_0::name_property(&f))
                .add_p(pool_3_0::uuid_property(&f))
                .add_p(pool_3_0::encrypted_property(&f))
                .add_p(pool_3_0::avail_actions_property(&f))
                .add_p(pool_3_8::key_descs_property(&f))
                .add_p(pool_3_8::clevis_infos_property(&f))
                .add_p(pool_3_0::has_cache_property(&f))
                .add_p(pool_3_0::alloc_size_property(&f))
                .add_p(pool_3_0::used_size_property(&f))
                .add_p(pool_3_0::total_size_property(&f))
                .add_p(pool_3_1::fs_limit_property(&f))
                .add_p(pool_3_1::enable_overprov_property(&f))
                .add_p(pool_3_1::no_alloc_space_property(&f))
                .add_p(pool_3_7::metadata_version_property(&f)),
        );

    let path = object_path.get_name().to_owned();
    let interfaces = get_pool_properties(name, uuid, pool);
    dbus_context.push_add(object_path, interfaces);
    path
}

/// Get the initial state of all properties associated with a pool object.
pub fn get_pool_properties(
    pool_name: &Name,
    pool_uuid: PoolUuid,
    pool: &dyn Pool,
) -> InterfacesAddedThreadSafe {
    initial_properties! {
        consts::POOL_INTERFACE_NAME_3_0 => {
            consts::POOL_NAME_PROP => shared::pool_name_prop(pool_name),
            consts::POOL_UUID_PROP => uuid_to_string!(pool_uuid),
            consts::POOL_ENCRYPTED_PROP => shared::pool_enc_prop(pool),
            consts::POOL_AVAIL_ACTIONS_PROP => shared::pool_avail_actions_prop(pool),
            consts::POOL_KEY_DESC_PROP => shared::pool_key_desc_prop(pool),
            consts::POOL_CLEVIS_INFO_PROP => shared::pool_clevis_info_prop(pool),
            consts::POOL_HAS_CACHE_PROP => shared::pool_has_cache_prop(pool),
            consts::POOL_ALLOC_SIZE_PROP => shared::pool_allocated_size(pool),
            consts::POOL_TOTAL_USED_PROP => shared::pool_used_size(pool),
            consts::POOL_TOTAL_SIZE_PROP => shared::pool_total_size(pool)
        },
        consts::POOL_INTERFACE_NAME_3_1 => {
            consts::POOL_NAME_PROP => shared::pool_name_prop(pool_name),
            consts::POOL_UUID_PROP => uuid_to_string!(pool_uuid),
            consts::POOL_ENCRYPTED_PROP => shared::pool_enc_prop(pool),
            consts::POOL_AVAIL_ACTIONS_PROP => shared::pool_avail_actions_prop(pool),
            consts::POOL_KEY_DESC_PROP => shared::pool_key_desc_prop(pool),
            consts::POOL_CLEVIS_INFO_PROP => shared::pool_clevis_info_prop(pool),
            consts::POOL_HAS_CACHE_PROP => shared::pool_has_cache_prop(pool),
            consts::POOL_ALLOC_SIZE_PROP => shared::pool_allocated_size(pool),
            consts::POOL_TOTAL_USED_PROP => shared::pool_used_size(pool),
            consts::POOL_TOTAL_SIZE_PROP => shared::pool_total_size(pool),
            consts::POOL_FS_LIMIT_PROP => shared::pool_fs_limit(pool),
            consts::POOL_OVERPROV_PROP => shared::pool_overprov_enabled(pool),
            consts::POOL_NO_ALLOCABLE_SPACE_PROP => shared::pool_no_alloc_space(pool)
        },
        consts::POOL_INTERFACE_NAME_3_2 => {
            consts::POOL_NAME_PROP => shared::pool_name_prop(pool_name),
            consts::POOL_UUID_PROP => uuid_to_string!(pool_uuid),
            consts::POOL_ENCRYPTED_PROP => shared::pool_enc_prop(pool),
            consts::POOL_AVAIL_ACTIONS_PROP => shared::pool_avail_actions_prop(pool),
            consts::POOL_KEY_DESC_PROP => shared::pool_key_desc_prop(pool),
            consts::POOL_CLEVIS_INFO_PROP => shared::pool_clevis_info_prop(pool),
            consts::POOL_HAS_CACHE_PROP => shared::pool_has_cache_prop(pool),
            consts::POOL_ALLOC_SIZE_PROP => shared::pool_allocated_size(pool),
            consts::POOL_TOTAL_USED_PROP => shared::pool_used_size(pool),
            consts::POOL_TOTAL_SIZE_PROP => shared::pool_total_size(pool),
            consts::POOL_FS_LIMIT_PROP => shared::pool_fs_limit(pool),
            consts::POOL_OVERPROV_PROP => shared::pool_overprov_enabled(pool),
            consts::POOL_NO_ALLOCABLE_SPACE_PROP => shared::pool_no_alloc_space(pool)
        },
        consts::POOL_INTERFACE_NAME_3_3 => {
            consts::POOL_NAME_PROP => shared::pool_name_prop(pool_name),
            consts::POOL_UUID_PROP => uuid_to_string!(pool_uuid),
            consts::POOL_ENCRYPTED_PROP => shared::pool_enc_prop(pool),
            consts::POOL_AVAIL_ACTIONS_PROP => shared::pool_avail_actions_prop(pool),
            consts::POOL_KEY_DESC_PROP => shared::pool_key_desc_prop(pool),
            consts::POOL_CLEVIS_INFO_PROP => shared::pool_clevis_info_prop(pool),
            consts::POOL_HAS_CACHE_PROP => shared::pool_has_cache_prop(pool),
            consts::POOL_ALLOC_SIZE_PROP => shared::pool_allocated_size(pool),
            consts::POOL_TOTAL_USED_PROP => shared::pool_used_size(pool),
            consts::POOL_TOTAL_SIZE_PROP => shared::pool_total_size(pool),
            consts::POOL_FS_LIMIT_PROP => shared::pool_fs_limit(pool),
            consts::POOL_OVERPROV_PROP => shared::pool_overprov_enabled(pool),
            consts::POOL_NO_ALLOCABLE_SPACE_PROP => shared::pool_no_alloc_space(pool)
        },
        consts::POOL_INTERFACE_NAME_3_4 => {
            consts::POOL_NAME_PROP => shared::pool_name_prop(pool_name),
            consts::POOL_UUID_PROP => uuid_to_string!(pool_uuid),
            consts::POOL_ENCRYPTED_PROP => shared::pool_enc_prop(pool),
            consts::POOL_AVAIL_ACTIONS_PROP => shared::pool_avail_actions_prop(pool),
            consts::POOL_KEY_DESC_PROP => shared::pool_key_desc_prop(pool),
            consts::POOL_CLEVIS_INFO_PROP => shared::pool_clevis_info_prop(pool),
            consts::POOL_HAS_CACHE_PROP => shared::pool_has_cache_prop(pool),
            consts::POOL_ALLOC_SIZE_PROP => shared::pool_allocated_size(pool),
            consts::POOL_TOTAL_USED_PROP => shared::pool_used_size(pool),
            consts::POOL_TOTAL_SIZE_PROP => shared::pool_total_size(pool),
            consts::POOL_FS_LIMIT_PROP => shared::pool_fs_limit(pool),
            consts::POOL_OVERPROV_PROP => shared::pool_overprov_enabled(pool),
            consts::POOL_NO_ALLOCABLE_SPACE_PROP => shared::pool_no_alloc_space(pool)
        },
        consts::POOL_INTERFACE_NAME_3_5 => {
            consts::POOL_NAME_PROP => shared::pool_name_prop(pool_name),
            consts::POOL_UUID_PROP => uuid_to_string!(pool_uuid),
            consts::POOL_ENCRYPTED_PROP => shared::pool_enc_prop(pool),
            consts::POOL_AVAIL_ACTIONS_PROP => shared::pool_avail_actions_prop(pool),
            consts::POOL_KEY_DESC_PROP => shared::pool_key_desc_prop(pool),
            consts::POOL_CLEVIS_INFO_PROP => shared::pool_clevis_info_prop(pool),
            consts::POOL_HAS_CACHE_PROP => shared::pool_has_cache_prop(pool),
            consts::POOL_ALLOC_SIZE_PROP => shared::pool_allocated_size(pool),
            consts::POOL_TOTAL_USED_PROP => shared::pool_used_size(pool),
            consts::POOL_TOTAL_SIZE_PROP => shared::pool_total_size(pool),
            consts::POOL_FS_LIMIT_PROP => shared::pool_fs_limit(pool),
            consts::POOL_OVERPROV_PROP => shared::pool_overprov_enabled(pool),
            consts::POOL_NO_ALLOCABLE_SPACE_PROP => shared::pool_no_alloc_space(pool)
        },
        consts::POOL_INTERFACE_NAME_3_6 => {
            consts::POOL_NAME_PROP => shared::pool_name_prop(pool_name),
            consts::POOL_UUID_PROP => uuid_to_string!(pool_uuid),
            consts::POOL_ENCRYPTED_PROP => shared::pool_enc_prop(pool),
            consts::POOL_AVAIL_ACTIONS_PROP => shared::pool_avail_actions_prop(pool),
            consts::POOL_KEY_DESC_PROP => shared::pool_key_desc_prop(pool),
            consts::POOL_CLEVIS_INFO_PROP => shared::pool_clevis_info_prop(pool),
            consts::POOL_HAS_CACHE_PROP => shared::pool_has_cache_prop(pool),
            consts::POOL_ALLOC_SIZE_PROP => shared::pool_allocated_size(pool),
            consts::POOL_TOTAL_USED_PROP => shared::pool_used_size(pool),
            consts::POOL_TOTAL_SIZE_PROP => shared::pool_total_size(pool),
            consts::POOL_FS_LIMIT_PROP => shared::pool_fs_limit(pool),
            consts::POOL_OVERPROV_PROP => shared::pool_overprov_enabled(pool),
            consts::POOL_NO_ALLOCABLE_SPACE_PROP => shared::pool_no_alloc_space(pool)
        },
        consts::POOL_INTERFACE_NAME_3_7 => {
            consts::POOL_NAME_PROP => shared::pool_name_prop(pool_name),
            consts::POOL_UUID_PROP => uuid_to_string!(pool_uuid),
            consts::POOL_ENCRYPTED_PROP => shared::pool_enc_prop(pool),
            consts::POOL_AVAIL_ACTIONS_PROP => shared::pool_avail_actions_prop(pool),
            consts::POOL_KEY_DESC_PROP => shared::pool_key_desc_prop(pool),
            consts::POOL_CLEVIS_INFO_PROP => shared::pool_clevis_info_prop(pool),
            consts::POOL_HAS_CACHE_PROP => shared::pool_has_cache_prop(pool),
            consts::POOL_ALLOC_SIZE_PROP => shared::pool_allocated_size(pool),
            consts::POOL_TOTAL_USED_PROP => shared::pool_used_size(pool),
            consts::POOL_TOTAL_SIZE_PROP => shared::pool_total_size(pool),
            consts::POOL_FS_LIMIT_PROP => shared::pool_fs_limit(pool),
            consts::POOL_OVERPROV_PROP => shared::pool_overprov_enabled(pool),
            consts::POOL_NO_ALLOCABLE_SPACE_PROP => shared::pool_no_alloc_space(pool)
        },
        consts::POOL_INTERFACE_NAME_3_8 => {
            consts::POOL_NAME_PROP => shared::pool_name_prop(pool_name),
            consts::POOL_UUID_PROP => uuid_to_string!(pool_uuid),
            consts::POOL_ENCRYPTED_PROP => shared::pool_enc_prop(pool),
            consts::POOL_AVAIL_ACTIONS_PROP => shared::pool_avail_actions_prop(pool),
            consts::POOL_KEY_DESCS_PROP => shared::pool_key_descs_prop(pool),
            consts::POOL_CLEVIS_INFOS_PROP => shared::pool_clevis_infos_prop(pool),
            consts::POOL_HAS_CACHE_PROP => shared::pool_has_cache_prop(pool),
            consts::POOL_ALLOC_SIZE_PROP => shared::pool_allocated_size(pool),
            consts::POOL_TOTAL_USED_PROP => shared::pool_used_size(pool),
            consts::POOL_TOTAL_SIZE_PROP => shared::pool_total_size(pool),
            consts::POOL_FS_LIMIT_PROP => shared::pool_fs_limit(pool),
            consts::POOL_OVERPROV_PROP => shared::pool_overprov_enabled(pool),
            consts::POOL_NO_ALLOCABLE_SPACE_PROP => shared::pool_no_alloc_space(pool),
            consts::POOL_METADATA_VERSION_PROP => shared::pool_metadata_version(pool)
        }
    }
}

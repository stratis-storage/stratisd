// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashSet, path::Path};

use either::Either;
use serde_json::Value;

use devicemapper::{Bytes, Sectors};

use crate::{
    engine::{
        engine::{BlockDev, Filesystem, Pool},
        strat_engine::pool::{v1, v2},
        types::{
            ActionAvailability, BlockDevTier, Clevis, CreateAction, DeleteAction, DevUuid,
            EncryptedDevice, EncryptionInfo, FilesystemUuid, GrowAction, InputEncryptionInfo, Key,
            KeyDescription, Name, OptionalTokenSlotInput, PoolDiff, PoolEncryptionInfo, PoolUuid,
            PropChangeAction, ReencryptedDevice, RegenAction, RenameAction, SetCreateAction,
            SetDeleteAction, StratSigblockVersion,
        },
    },
    stratis::StratisResult,
};

#[derive(Debug)]
pub enum AnyPool {
    V1(v1::StratPool),
    V2(v2::StratPool),
}

impl Pool for AnyPool {
    fn init_cache(
        &mut self,
        pool_uuid: PoolUuid,
        pool_name: &str,
        blockdevs: &[&Path],
        supports_encrypted: bool,
    ) -> StratisResult<SetCreateAction<DevUuid>> {
        match self {
            AnyPool::V1(p) => p.init_cache(pool_uuid, pool_name, blockdevs, supports_encrypted),
            AnyPool::V2(p) => p.init_cache(pool_uuid, pool_name, blockdevs, supports_encrypted),
        }
    }

    fn bind_clevis(
        &mut self,
        token_slot: OptionalTokenSlotInput,
        pin: &str,
        clevis_info: &Value,
    ) -> StratisResult<CreateAction<(Clevis, u32)>> {
        match self {
            AnyPool::V1(p) => p.bind_clevis(token_slot, pin, clevis_info),
            AnyPool::V2(p) => p.bind_clevis(token_slot, pin, clevis_info),
        }
    }

    fn bind_keyring(
        &mut self,
        token_slot: OptionalTokenSlotInput,
        key_description: &KeyDescription,
    ) -> StratisResult<CreateAction<(Key, u32)>> {
        match self {
            AnyPool::V1(p) => p.bind_keyring(token_slot, key_description),
            AnyPool::V2(p) => p.bind_keyring(token_slot, key_description),
        }
    }

    fn rebind_keyring(
        &mut self,
        token_slot: Option<u32>,
        new_key_desc: &KeyDescription,
    ) -> StratisResult<RenameAction<Key>> {
        match self {
            AnyPool::V1(p) => p.rebind_keyring(token_slot, new_key_desc),
            AnyPool::V2(p) => p.rebind_keyring(token_slot, new_key_desc),
        }
    }

    fn rebind_clevis(&mut self, token_slot: Option<u32>) -> StratisResult<RegenAction> {
        match self {
            AnyPool::V1(p) => p.rebind_clevis(token_slot),
            AnyPool::V2(p) => p.rebind_clevis(token_slot),
        }
    }

    fn unbind_keyring(&mut self, token_slot: Option<u32>) -> StratisResult<DeleteAction<Key>> {
        match self {
            AnyPool::V1(p) => p.unbind_keyring(token_slot),
            AnyPool::V2(p) => p.unbind_keyring(token_slot),
        }
    }

    fn unbind_clevis(&mut self, token_slot: Option<u32>) -> StratisResult<DeleteAction<Clevis>> {
        match self {
            AnyPool::V1(p) => p.unbind_clevis(token_slot),
            AnyPool::V2(p) => p.unbind_clevis(token_slot),
        }
    }

    fn create_filesystems<'a>(
        &mut self,
        pool_name: &str,
        pool_uuid: PoolUuid,
        specs: &[(&'a str, Option<Bytes>, Option<Bytes>)],
    ) -> StratisResult<SetCreateAction<(&'a str, FilesystemUuid, Sectors)>> {
        match self {
            AnyPool::V1(p) => p.create_filesystems(pool_name, pool_uuid, specs),
            AnyPool::V2(p) => p.create_filesystems(pool_name, pool_uuid, specs),
        }
    }

    fn add_blockdevs(
        &mut self,
        pool_uuid: PoolUuid,
        pool_name: &str,
        paths: &[&Path],
        tier: BlockDevTier,
    ) -> StratisResult<(SetCreateAction<DevUuid>, Option<PoolDiff>)> {
        match self {
            AnyPool::V1(p) => p.add_blockdevs(pool_uuid, pool_name, paths, tier),
            AnyPool::V2(p) => p.add_blockdevs(pool_uuid, pool_name, paths, tier),
        }
    }

    fn destroy_filesystems(
        &mut self,
        pool_name: &str,
        fs_uuids: &HashSet<FilesystemUuid>,
    ) -> StratisResult<SetDeleteAction<FilesystemUuid, (FilesystemUuid, Option<FilesystemUuid>)>>
    {
        match self {
            AnyPool::V1(p) => p.destroy_filesystems(pool_name, fs_uuids),
            AnyPool::V2(p) => p.destroy_filesystems(pool_name, fs_uuids),
        }
    }

    fn rename_filesystem(
        &mut self,
        pool_name: &str,
        uuid: FilesystemUuid,
        new_name: &str,
    ) -> StratisResult<RenameAction<FilesystemUuid>> {
        match self {
            AnyPool::V1(p) => p.rename_filesystem(pool_name, uuid, new_name),
            AnyPool::V2(p) => p.rename_filesystem(pool_name, uuid, new_name),
        }
    }

    fn snapshot_filesystem<'a>(
        &'a mut self,
        pool_name: &str,
        pool_uuid: PoolUuid,
        origin_uuid: FilesystemUuid,
        snapshot_name: &str,
    ) -> StratisResult<CreateAction<(FilesystemUuid, &'a mut dyn Filesystem)>> {
        match self {
            AnyPool::V1(p) => {
                p.snapshot_filesystem(pool_name, pool_uuid, origin_uuid, snapshot_name)
            }
            AnyPool::V2(p) => {
                p.snapshot_filesystem(pool_name, pool_uuid, origin_uuid, snapshot_name)
            }
        }
    }

    fn total_physical_size(&self) -> Sectors {
        match self {
            AnyPool::V1(p) => p.total_physical_size(),
            AnyPool::V2(p) => p.total_physical_size(),
        }
    }

    fn total_allocated_size(&self) -> Sectors {
        match self {
            AnyPool::V1(p) => p.total_allocated_size(),
            AnyPool::V2(p) => p.total_allocated_size(),
        }
    }

    fn total_physical_used(&self) -> Option<Sectors> {
        match self {
            AnyPool::V1(p) => p.total_physical_used(),
            AnyPool::V2(p) => p.total_physical_used(),
        }
    }

    fn filesystems(&self) -> Vec<(Name, FilesystemUuid, &dyn Filesystem)> {
        match self {
            AnyPool::V1(p) => p.filesystems(),
            AnyPool::V2(p) => p.filesystems(),
        }
    }

    fn get_filesystem(&self, uuid: FilesystemUuid) -> Option<(Name, &dyn Filesystem)> {
        match self {
            AnyPool::V1(p) => p.get_filesystem(uuid),
            AnyPool::V2(p) => p.get_filesystem(uuid),
        }
    }

    fn get_filesystem_by_name(&self, fs_name: &Name) -> Option<(FilesystemUuid, &dyn Filesystem)> {
        match self {
            AnyPool::V1(p) => p.get_filesystem_by_name(fs_name),
            AnyPool::V2(p) => p.get_filesystem_by_name(fs_name),
        }
    }

    fn blockdevs(&self) -> Vec<(DevUuid, BlockDevTier, &dyn BlockDev)> {
        match self {
            AnyPool::V1(p) => <v1::StratPool as Pool>::blockdevs(p),
            AnyPool::V2(p) => <v2::StratPool as Pool>::blockdevs(p),
        }
    }

    fn get_blockdev(&self, uuid: DevUuid) -> Option<(BlockDevTier, &dyn BlockDev)> {
        match self {
            AnyPool::V1(p) => p.get_blockdev(uuid),
            AnyPool::V2(p) => p.get_blockdev(uuid),
        }
    }

    fn get_mut_blockdev(
        &mut self,
        uuid: DevUuid,
    ) -> StratisResult<Option<(BlockDevTier, &mut dyn BlockDev)>> {
        match self {
            AnyPool::V1(p) => p.get_mut_blockdev(uuid),
            AnyPool::V2(p) => p.get_mut_blockdev(uuid),
        }
    }

    fn set_blockdev_user_info(
        &mut self,
        pool_name: &str,
        uuid: DevUuid,
        user_info: Option<&str>,
    ) -> StratisResult<RenameAction<DevUuid>> {
        match self {
            AnyPool::V1(p) => p.set_blockdev_user_info(pool_name, uuid, user_info),
            AnyPool::V2(p) => p.set_blockdev_user_info(pool_name, uuid, user_info),
        }
    }

    fn has_cache(&self) -> bool {
        match self {
            AnyPool::V1(p) => p.has_cache(),
            AnyPool::V2(p) => p.has_cache(),
        }
    }

    fn is_encrypted(&self) -> bool {
        match self {
            AnyPool::V1(p) => p.is_encrypted(),
            AnyPool::V2(p) => p.is_encrypted(),
        }
    }

    fn encryption_info_legacy(&self) -> Option<PoolEncryptionInfo> {
        match self {
            AnyPool::V1(p) => p.encryption_info_legacy(),
            AnyPool::V2(p) => p.encryption_info_legacy(),
        }
    }

    fn encryption_info(&self) -> Option<Either<EncryptionInfo, PoolEncryptionInfo>> {
        match self {
            AnyPool::V1(p) => p.encryption_info(),
            AnyPool::V2(p) => p.encryption_info(),
        }
    }

    fn avail_actions(&self) -> ActionAvailability {
        match self {
            AnyPool::V1(p) => p.avail_actions(),
            AnyPool::V2(p) => p.avail_actions(),
        }
    }

    fn fs_limit(&self) -> u64 {
        match self {
            AnyPool::V1(p) => p.fs_limit(),
            AnyPool::V2(p) => p.fs_limit(),
        }
    }

    fn set_fs_limit(
        &mut self,
        pool_name: &Name,
        pool_uuid: PoolUuid,
        new_limit: u64,
    ) -> StratisResult<()> {
        match self {
            AnyPool::V1(p) => p.set_fs_limit(pool_name, pool_uuid, new_limit),
            AnyPool::V2(p) => p.set_fs_limit(pool_name, pool_uuid, new_limit),
        }
    }

    fn overprov_enabled(&self) -> bool {
        match self {
            AnyPool::V1(p) => p.overprov_enabled(),
            AnyPool::V2(p) => p.overprov_enabled(),
        }
    }

    fn set_overprov_mode(&mut self, pool_name: &Name, enabled: bool) -> StratisResult<()> {
        match self {
            AnyPool::V1(p) => p.set_overprov_mode(pool_name, enabled),
            AnyPool::V2(p) => p.set_overprov_mode(pool_name, enabled),
        }
    }

    fn out_of_alloc_space(&self) -> bool {
        match self {
            AnyPool::V1(p) => p.out_of_alloc_space(),
            AnyPool::V2(p) => p.out_of_alloc_space(),
        }
    }

    fn grow_physical(
        &mut self,
        name: &Name,
        pool_uuid: PoolUuid,
        device: DevUuid,
    ) -> StratisResult<(GrowAction<(PoolUuid, DevUuid)>, Option<PoolDiff>)> {
        match self {
            AnyPool::V1(p) => p.grow_physical(name, pool_uuid, device),
            AnyPool::V2(p) => p.grow_physical(name, pool_uuid, device),
        }
    }

    fn set_fs_size_limit(
        &mut self,
        fs: FilesystemUuid,
        limit: Option<Bytes>,
    ) -> StratisResult<PropChangeAction<Option<Sectors>>> {
        match self {
            AnyPool::V1(p) => p.set_fs_size_limit(fs, limit),
            AnyPool::V2(p) => p.set_fs_size_limit(fs, limit),
        }
    }

    fn encrypt_pool(
        &mut self,
        name: &Name,
        pool_uuid: PoolUuid,
        encryption_info: &InputEncryptionInfo,
    ) -> StratisResult<CreateAction<EncryptedDevice>> {
        match self {
            AnyPool::V1(p) => p.encrypt_pool(name, pool_uuid, encryption_info),
            AnyPool::V2(p) => p.encrypt_pool(name, pool_uuid, encryption_info),
        }
    }

    fn reencrypt_pool(&mut self) -> StratisResult<ReencryptedDevice> {
        match self {
            AnyPool::V1(p) => p.reencrypt_pool(),
            AnyPool::V2(p) => p.reencrypt_pool(),
        }
    }

    fn decrypt_pool(
        &mut self,
        name: &Name,
        pool_uuid: PoolUuid,
    ) -> StratisResult<DeleteAction<EncryptedDevice>> {
        match self {
            AnyPool::V1(p) => p.decrypt_pool(name, pool_uuid),
            AnyPool::V2(p) => p.decrypt_pool(name, pool_uuid),
        }
    }

    fn current_metadata(&self, pool_name: &Name) -> StratisResult<String> {
        match self {
            AnyPool::V1(p) => p.current_metadata(pool_name),
            AnyPool::V2(p) => p.current_metadata(pool_name),
        }
    }

    fn last_metadata(&self) -> StratisResult<String> {
        match self {
            AnyPool::V1(p) => p.last_metadata(),
            AnyPool::V2(p) => p.last_metadata(),
        }
    }

    fn metadata_version(&self) -> StratSigblockVersion {
        match self {
            AnyPool::V1(p) => p.metadata_version(),
            AnyPool::V2(p) => p.metadata_version(),
        }
    }

    fn current_fs_metadata(&self, fs_name: Option<&str>) -> StratisResult<String> {
        match self {
            AnyPool::V1(p) => p.current_fs_metadata(fs_name),
            AnyPool::V2(p) => p.current_fs_metadata(fs_name),
        }
    }

    fn last_fs_metadata(&self, fs_name: Option<&str>) -> StratisResult<String> {
        match self {
            AnyPool::V1(p) => p.last_fs_metadata(fs_name),
            AnyPool::V2(p) => p.last_fs_metadata(fs_name),
        }
    }

    fn set_fs_merge_scheduled(
        &mut self,
        fs_uuid: FilesystemUuid,
        new_scheduled: bool,
    ) -> StratisResult<PropChangeAction<bool>> {
        match self {
            AnyPool::V1(p) => p.set_fs_merge_scheduled(fs_uuid, new_scheduled),
            AnyPool::V2(p) => p.set_fs_merge_scheduled(fs_uuid, new_scheduled),
        }
    }
}

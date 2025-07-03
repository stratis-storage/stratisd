// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod dispatch;
#[cfg(feature = "extras")]
pub mod inspection;
pub mod v1;
pub mod v2;

pub use dispatch::AnyPool;

use std::{collections::HashSet, path::Path};

use devicemapper::{Bytes, Sectors};
use either::Either;
use serde_json::Value;

use crate::{
    engine::{
        engine::{BlockDev, Filesystem, Pool},
        strat_engine::{backstore::blockdev::LockedBlockdev, thinpool::LockedFilesystem},
        types::{
            ActionAvailability, BlockDevTier, Clevis, CreateAction, DeleteAction, DevUuid,
            EncryptionInfo, FilesystemUuid, GrowAction, Key, KeyDescription, Name,
            OptionalTokenSlotInput, PoolDiff, PoolEncryptionInfo, PoolUuid, RegenAction,
            RenameAction, SetCreateAction, SetDeleteAction, StratSigblockVersion,
        },
        PropChangeAction,
    },
    stratis::{StratisError, StratisResult},
};

const ERROR_MESSAGE: &str =
    "This pool is currently locked and cannot be modified or actively queried";

/// Represents a locked pool with cached information.
#[derive(Debug)]
pub struct LockedPool {
    blockdevs: Vec<(DevUuid, BlockDevTier, LockedBlockdev)>,
    filesystems: Vec<(Name, FilesystemUuid, LockedFilesystem)>,
    total_physical_size: Sectors,
    total_allocated_size: Sectors,
    total_physical_used: Option<Sectors>,
    has_cache: bool,
    is_encrypted: bool,
    encryption_info_legacy: Option<PoolEncryptionInfo>,
    encryption_info: Option<Either<EncryptionInfo, PoolEncryptionInfo>>,
    avail_actions: ActionAvailability,
    fs_limit: u64,
    overprov_enabled: bool,
    out_of_alloc_space: bool,
    last_metadata: StratisResult<String>,
    metadata_version: StratSigblockVersion,
    free_token_slots: Option<u8>,
    volume_key_is_loaded: StratisResult<bool>,
}

impl LockedPool {
    fn from_pool(p: &dyn Pool, pool_uuid: PoolUuid) -> Self {
        LockedPool {
            blockdevs: p
                .blockdevs()
                .into_iter()
                .map(|(uuid, tier, bd)| (uuid, tier, LockedBlockdev::from_bd(bd)))
                .collect::<Vec<_>>(),
            filesystems: p
                .filesystems()
                .into_iter()
                .map(|(name, uuid, fs)| (name, uuid, LockedFilesystem::from_fs(fs)))
                .collect::<Vec<_>>(),
            total_physical_size: p.total_physical_size(),
            total_allocated_size: p.total_allocated_size(),
            total_physical_used: p.total_physical_used(),
            has_cache: p.has_cache(),
            is_encrypted: p.is_encrypted(),
            encryption_info_legacy: p.encryption_info_legacy(),
            encryption_info: p.encryption_info(),
            avail_actions: p.avail_actions(),
            fs_limit: p.fs_limit(),
            overprov_enabled: p.overprov_enabled(),
            out_of_alloc_space: p.out_of_alloc_space(),
            last_metadata: p.last_metadata(),
            metadata_version: p.metadata_version(),
            free_token_slots: p.free_token_slots(),
            volume_key_is_loaded: p.volume_key_is_loaded(pool_uuid),
        }
    }
}

impl Pool for LockedPool {
    fn init_cache(
        &mut self,
        _: PoolUuid,
        _: &str,
        _: &[&Path],
        _: bool,
    ) -> StratisResult<SetCreateAction<DevUuid>> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn create_filesystems<'b>(
        &mut self,
        _: &str,
        _: PoolUuid,
        _: &[(&'b str, Option<Bytes>, Option<Bytes>)],
    ) -> StratisResult<SetCreateAction<(&'b str, FilesystemUuid, Sectors)>> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn add_blockdevs(
        &mut self,
        _: PoolUuid,
        _: &str,
        _: &[&Path],
        _: BlockDevTier,
    ) -> StratisResult<(SetCreateAction<DevUuid>, Option<PoolDiff>)> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn bind_clevis(
        &mut self,
        _: &Name,
        _: OptionalTokenSlotInput,
        _: &str,
        _: &Value,
    ) -> StratisResult<CreateAction<(Clevis, u32)>> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn bind_keyring(
        &mut self,
        _: &Name,
        _: OptionalTokenSlotInput,
        _: &KeyDescription,
    ) -> StratisResult<CreateAction<(Key, u32)>> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn unbind_keyring(&mut self, _: &Name, _: Option<u32>) -> StratisResult<DeleteAction<Key>> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn unbind_clevis(&mut self, _: &Name, _: Option<u32>) -> StratisResult<DeleteAction<Clevis>> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn rebind_keyring(
        &mut self,
        _: Option<u32>,
        _: &KeyDescription,
    ) -> StratisResult<RenameAction<Key>> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn rebind_clevis(&mut self, _: Option<u32>) -> StratisResult<RegenAction> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn destroy_filesystems(
        &mut self,
        _: &str,
        _: &HashSet<FilesystemUuid>,
    ) -> StratisResult<SetDeleteAction<FilesystemUuid, (FilesystemUuid, Option<FilesystemUuid>)>>
    {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn rename_filesystem(
        &mut self,
        _: &str,
        _: FilesystemUuid,
        _: &str,
    ) -> StratisResult<RenameAction<FilesystemUuid>> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn snapshot_filesystem(
        &mut self,
        _: &str,
        _: PoolUuid,
        _: FilesystemUuid,
        _: &str,
    ) -> StratisResult<CreateAction<(FilesystemUuid, &mut dyn Filesystem)>> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn total_physical_size(&self) -> Sectors {
        self.total_physical_size
    }

    fn total_allocated_size(&self) -> Sectors {
        self.total_allocated_size
    }

    fn total_physical_used(&self) -> Option<Sectors> {
        self.total_physical_used.clone()
    }

    fn filesystems(&self) -> Vec<(Name, FilesystemUuid, &dyn Filesystem)> {
        self.filesystems
            .iter()
            .map(|(name, uuid, x)| (name.clone(), *uuid, x as &dyn Filesystem))
            .collect()
    }

    fn get_filesystem(&self, fs_uuid: FilesystemUuid) -> Option<(Name, &dyn Filesystem)> {
        self.filesystems.iter().find_map(|(name, uuid, fs)| {
            if fs_uuid == *uuid {
                Some((name.clone(), fs as &dyn Filesystem))
            } else {
                None
            }
        })
    }

    fn get_filesystem_by_name(&self, fs_name: &Name) -> Option<(FilesystemUuid, &dyn Filesystem)> {
        self.filesystems.iter().find_map(|(name, uuid, fs)| {
            if fs_name == name {
                Some((*uuid, fs as &dyn Filesystem))
            } else {
                None
            }
        })
    }

    fn blockdevs(&self) -> Vec<(DevUuid, BlockDevTier, &dyn BlockDev)> {
        self.blockdevs
            .iter()
            .map(|(uuid, tier, bd)| (*uuid, *tier, bd as &dyn BlockDev))
            .collect::<Vec<_>>()
    }

    fn get_blockdev(&self, dev_uuid: DevUuid) -> Option<(BlockDevTier, &dyn BlockDev)> {
        self.blockdevs.iter().find_map(|(uuid, tier, bd)| {
            if *uuid == dev_uuid {
                Some((*tier, bd as &dyn BlockDev))
            } else {
                None
            }
        })
    }

    fn get_mut_blockdev(
        &mut self,
        _: DevUuid,
    ) -> StratisResult<Option<(BlockDevTier, &mut dyn BlockDev)>> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn set_blockdev_user_info(
        &mut self,
        _: &str,
        _: DevUuid,
        _: Option<&str>,
    ) -> StratisResult<RenameAction<DevUuid>> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn has_cache(&self) -> bool {
        self.has_cache
    }

    fn is_encrypted(&self) -> bool {
        self.is_encrypted
    }

    fn encryption_info_legacy(&self) -> Option<PoolEncryptionInfo> {
        self.encryption_info_legacy.clone()
    }

    fn encryption_info(&self) -> Option<Either<EncryptionInfo, PoolEncryptionInfo>> {
        self.encryption_info.clone()
    }

    fn avail_actions(&self) -> ActionAvailability {
        self.avail_actions.clone()
    }

    fn fs_limit(&self) -> u64 {
        self.fs_limit
    }

    fn set_fs_limit(&mut self, _: &Name, _: PoolUuid, _: u64) -> StratisResult<()> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn overprov_enabled(&self) -> bool {
        self.overprov_enabled
    }

    fn set_overprov_mode(&mut self, _: &Name, _: bool) -> StratisResult<()> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn out_of_alloc_space(&self) -> bool {
        self.out_of_alloc_space
    }

    fn grow_physical(
        &mut self,
        _: &Name,
        _: PoolUuid,
        _: DevUuid,
    ) -> StratisResult<(GrowAction<(PoolUuid, DevUuid)>, Option<PoolDiff>)> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn set_fs_size_limit(
        &mut self,
        _: FilesystemUuid,
        _: Option<Bytes>,
    ) -> StratisResult<PropChangeAction<Option<Sectors>>> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn current_metadata(&self, _: &Name) -> StratisResult<String> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn last_metadata(&self) -> StratisResult<String> {
        self.last_metadata.clone()
    }

    fn metadata_version(&self) -> StratSigblockVersion {
        self.metadata_version
    }

    fn current_fs_metadata(&self, _: Option<&str>) -> StratisResult<String> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn last_fs_metadata(&self, _: Option<&str>) -> StratisResult<String> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn set_fs_merge_scheduled(
        &mut self,
        _: FilesystemUuid,
        _: bool,
    ) -> StratisResult<PropChangeAction<bool>> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }

    fn free_token_slots(&self) -> Option<u8> {
        self.free_token_slots
    }

    fn volume_key_is_loaded(&self, _: PoolUuid) -> StratisResult<bool> {
        self.volume_key_is_loaded.clone()
    }

    fn load_volume_key(&mut self, _: PoolUuid) -> StratisResult<bool> {
        Err(StratisError::Msg(ERROR_MESSAGE.to_string()))
    }
}

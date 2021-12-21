// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    os::unix::io::RawFd,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard};

use devicemapper::{Bytes, Sectors};

use crate::{
    engine::{
        structures::{
            AllLockReadGuard, AllLockWriteGuard, ExclusiveGuard, SharedGuard, SomeLockReadGuard,
            SomeLockWriteGuard,
        },
        types::{
            ActionAvailability, BlockDevTier, Clevis, CreateAction, DeleteAction, DevUuid,
            EncryptionInfo, FilesystemUuid, Key, KeyDescription, LockKey, LockedPoolInfo,
            MappingCreateAction, MappingDeleteAction, Name, PoolEncryptionInfo, PoolUuid,
            RegenAction, RenameAction, ReportType, SetCreateAction, SetDeleteAction,
            SetUnlockAction, StratFilesystemDiff, ThinPoolDiff, UdevEngineEvent, UnlockMethod,
        },
    },
    stratis::StratisResult,
};

pub const DEV_PATH: &str = "/dev/stratis";
/// The maximum size of pool passphrases stored in the kernel keyring
pub const MAX_STRATIS_PASS_SIZE: usize = 512 / 8;

pub trait KeyActions {
    /// Set a key in the kernel keyring. The output is an idempotent return type
    /// containing a `bool` which indicates whether a key with the requested
    /// key description was in the keyring and the key data was updated.
    ///
    /// Successful return values:
    /// * `Ok(MappingCreateAction::Identity)`: The key was already in the keyring
    /// with the appropriate key description and key data.
    /// * `Ok(MappingCreateAction::Created(_))`: The key was newly added to the
    /// keyring.
    /// * `Ok(MappingCreateAction::Changed)`: The key description was already present
    /// in the keyring but the key data was updated.
    fn set(
        &mut self,
        key_desc: &KeyDescription,
        key_fd: RawFd,
    ) -> StratisResult<MappingCreateAction<Key>>;

    /// Return a list of all key descriptions of keys added to the keyring by
    /// Stratis that are still valid.
    fn list(&self) -> StratisResult<Vec<KeyDescription>>;

    /// Unset a key with the given key description in the root persistent kernel
    /// keyring.
    fn unset(&mut self, key_desc: &KeyDescription) -> StratisResult<MappingDeleteAction<Key>>;
}

/// An interface for reporting internal engine state.
pub trait Report {
    /// Supported engine state report.
    ///
    /// NOTE: The JSON schema for this report is not guaranteed to be stable.
    fn engine_state_report(&self) -> Value;

    /// Unsupported reports. The available reports and JSON schemas of these reports may change.
    fn get_report(&self, report_type: ReportType) -> Value;
}

pub trait Filesystem: Debug {
    /// path of the device node
    fn devnode(&self) -> PathBuf;

    /// When the filesystem was created.
    fn created(&self) -> DateTime<Utc>;

    /// path to mount the filesystem by
    fn path_to_mount_filesystem(&self, pool_name: &str, fs_name: &str) -> PathBuf;

    /// The amount of data stored on the filesystem, including overhead.
    fn used(&self) -> StratisResult<Bytes>;

    /// Get the size of the filesystem in bytes.
    fn size(&self) -> Bytes;
}

pub trait BlockDev: Debug {
    /// Get the device path for the block device.
    fn devnode(&self) -> &Path;

    /// Get the path to the device on which the Stratis metadata is stored.
    fn metadata_path(&self) -> &Path;

    /// Get the user-settable string associated with this blockdev.
    fn user_info(&self) -> Option<&str>;

    /// Get the hardware ID for this blockdev.
    fn hardware_info(&self) -> Option<&str>;

    /// The time that this blockdev was initialized by Stratis, rounded down
    /// to the nearest second.
    fn initialization_time(&self) -> DateTime<Utc>;

    /// The total size of the device, including space not usable for data.
    fn size(&self) -> Sectors;

    /// Get the status of whether a block device is encrypted or not.
    fn is_encrypted(&self) -> bool;
}

pub trait Pool: Debug + Send + Sync {
    /// Filesystem type associated with this engine type.
    type Filesystem: Filesystem;
    /// Block device type associated with this engine type.
    type BlockDev: BlockDev;

    /// Initialize the cache with the provided cache block devices.
    /// Returns a list of the the block devices that were actually added as cache
    /// devices. In practice, this will have three types of return values:
    /// * An error if the cache has already been initialized with a different set
    /// of block devices.
    /// * `SetCreateAction::Identity` if the cache has already been initialized with
    /// the same set of block devices.
    /// * `SetCreateAction::Created` containing all provided block devices if the
    /// cache has not yet been initialized.
    ///
    /// This ensures the contract of providing a truly idempotent API as the cache
    /// can only be initialized once and if an attempt is made to initialize it
    /// twice with different sets of block devices, the user should be notified
    /// of their error.
    fn init_cache(
        &mut self,
        pool_uuid: PoolUuid,
        pool_name: &str,
        blockdevs: &[&Path],
    ) -> StratisResult<SetCreateAction<DevUuid>>;

    /// Creates the filesystems specified by specs.
    /// Returns a list of the names of filesystems actually created.
    /// Returns an error if any of the specified names are already in use
    /// for filesystems in this pool. If the same name is passed multiple
    /// times, the size associated with the last item is used.
    fn create_filesystems<'a, 'b>(
        &'a mut self,
        pool_name: &str,
        pool_uuid: PoolUuid,
        specs: &[(&'b str, Option<Bytes>)],
    ) -> StratisResult<SetCreateAction<(&'b str, FilesystemUuid, Sectors)>>;

    /// Adds blockdevs specified by paths to pool.
    /// Returns a list of uuids corresponding to devices actually added.
    /// Returns an error if a blockdev can not be added because it is owned
    /// or there was an error while reading or writing a blockdev.
    /// Also return an error if the tier specified is Cache, and the cache
    /// is not yet initialized.
    fn add_blockdevs(
        &mut self,
        pool_uuid: PoolUuid,
        pool_name: &str,
        paths: &[&Path],
        tier: BlockDevTier,
    ) -> StratisResult<SetCreateAction<DevUuid>>;

    /// Bind all devices in the given pool for automated unlocking
    /// using clevis.
    fn bind_clevis(
        &mut self,
        pin: &str,
        clevis_info: &Value,
    ) -> StratisResult<CreateAction<Clevis>>;

    /// Unbind all devices in the given pool from using clevis.
    fn unbind_clevis(&mut self) -> StratisResult<DeleteAction<Clevis>>;

    /// Bind all devices in the given pool for unlocking using a passphrase
    /// in the kernel keyring.
    fn bind_keyring(&mut self, key_desc: &KeyDescription) -> StratisResult<CreateAction<Key>>;

    /// Unbind all devices in the given pool from the registered keyring passphrase.
    fn unbind_keyring(&mut self) -> StratisResult<DeleteAction<Key>>;

    /// Change the key description and passphrase associated with a pool.
    fn rebind_keyring(&mut self, new_key_desc: &KeyDescription)
        -> StratisResult<RenameAction<Key>>;

    /// Regenerate the Clevis bindings associated with a pool.
    fn rebind_clevis(&mut self) -> StratisResult<RegenAction>;

    /// Ensures that all designated filesystems are gone from pool.
    /// Returns a list of the filesystems found, and actually destroyed.
    /// This list will be a subset of the uuids passed in fs_uuids.
    /// Precondition: All filesystems given must be unmounted.
    fn destroy_filesystems<'a>(
        &'a mut self,
        pool_name: &str,
        fs_uuids: &[FilesystemUuid],
    ) -> StratisResult<SetDeleteAction<FilesystemUuid>>;

    /// Rename filesystem
    /// Rename pool with uuid to new_name.
    /// Raises an error if the mapping can't be applied because
    /// new_name is already in use.
    /// The result indicate whether an action was performed, and if not, why.
    fn rename_filesystem(
        &mut self,
        pool_name: &str,
        uuid: FilesystemUuid,
        new_name: &str,
    ) -> StratisResult<RenameAction<FilesystemUuid>>;

    /// Snapshot filesystem
    /// Create a CoW snapshot of the origin
    fn snapshot_filesystem(
        &mut self,
        pool_name: &str,
        pool_uuid: PoolUuid,
        origin_uuid: FilesystemUuid,
        snapshot_name: &str,
    ) -> StratisResult<CreateAction<(FilesystemUuid, &mut Self::Filesystem)>>;

    /// The total number of Sectors belonging to this pool.
    /// There are no exclusions, so this number includes overhead sectors
    /// of all sorts, sectors allocated for every sort of metadata by
    /// Stratis or devicemapper and therefore not available to the user for
    /// storing their data. There is no larger physical size number that can be
    /// associated with a pool.
    fn total_physical_size(&self) -> Sectors;

    /// The total number of Sectors of physical storage that have been allocated
    /// in this pool.
    /// There are no exclusions, so this number includes overhead sectors
    /// of all sorts, sectors allocated for every sort of metadata by
    /// Stratis or devicemapper and therefore not available to the user for
    /// storing their data.
    ///
    /// self.total_allocated_size() <= self.total_physical_size() as no more
    /// physical space can be allocated for the pool than is available on
    /// the block devices.
    fn total_allocated_size(&self) -> Sectors;

    /// The number of Sectors in this pool that are currently in use by the
    /// pool for some purpose, and therefore not available for future use,
    /// by any subcomponent of Stratis, either for internal managment or to
    /// store user data.
    fn total_physical_used(&self) -> StratisResult<Sectors>;

    /// Get all the filesystems belonging to this pool.
    fn filesystems(&self) -> Vec<(Name, FilesystemUuid, &Self::Filesystem)>;

    /// Get the filesystem in this pool with this UUID.
    fn get_filesystem(&self, uuid: FilesystemUuid) -> Option<(Name, &Self::Filesystem)>;

    /// Get the filesystem in this pool with this name.
    fn get_filesystem_by_name(&self, name: &Name) -> Option<(FilesystemUuid, &Self::Filesystem)>;

    /// Get _all_ the blockdevs that belong to this pool.
    /// All really means all. For example, it does not exclude cache blockdevs.
    fn blockdevs(&self) -> Vec<(DevUuid, BlockDevTier, &Self::BlockDev)>;

    /// Get the blockdev in this pool with this UUID.
    fn get_blockdev(&self, uuid: DevUuid) -> Option<(BlockDevTier, &Self::BlockDev)>;

    /// Set the user-settable string associated with the blockdev specified
    /// by the uuid.
    fn set_blockdev_user_info(
        &mut self,
        pool_name: &str,
        uuid: DevUuid,
        user_info: Option<&str>,
    ) -> StratisResult<RenameAction<DevUuid>>;

    /// true if the pool has a cache, otherwise false
    fn has_cache(&self) -> bool;

    /// Determine if the pool's data is encrypted
    fn is_encrypted(&self) -> bool;

    /// Get all encryption information for this pool.
    fn encryption_info(&self) -> Option<PoolEncryptionInfo>;

    /// Get the pool state for the given pool. The state indicates which actions
    /// will be disabled or enabled. Disabled actions are triggered by failures
    /// caught by stratisd.
    fn avail_actions(&self) -> ActionAvailability;
}

#[async_trait]
pub trait Engine: Debug + Report + Send + Sync {
    /// Pool type associated with this engine type.
    type Pool: Pool;
    /// Key handling type associated with this engine type.
    type KeyActions: KeyActions;

    /// Create a Stratis pool.
    /// Returns the UUID of the newly created pool.
    async fn create_pool(
        &self,
        name: &str,
        blockdev_paths: &[&Path],
        encryption_info: Option<&EncryptionInfo>,
    ) -> StratisResult<CreateAction<PoolUuid>>;

    /// Handle a libudev event.
    /// If the handling action resulted in pool creation, return the pool
    /// and its UUID.
    ///
    /// Precondition: the subsystem of the device evented on is "block".
    async fn handle_events(
        &self,
        event: Vec<UdevEngineEvent>,
    ) -> Vec<SomeLockReadGuard<PoolUuid, Self::Pool>>;

    /// Destroy a pool.
    /// Ensures that the pool of the given UUID is absent on completion.
    /// Returns true if some action was necessary, otherwise false.
    async fn destroy_pool(&self, uuid: PoolUuid) -> StratisResult<DeleteAction<PoolUuid>>;

    /// Rename pool with uuid to new_name.
    /// Raises an error if the mapping can't be applied because
    /// new_name is already in use.
    /// Returns true if it was necessary to perform an action, false if not.
    async fn rename_pool(
        &self,
        uuid: PoolUuid,
        new_name: &str,
    ) -> StratisResult<RenameAction<PoolUuid>>;

    /// Unlock all encrypted devices registered under a given pool UUID.
    /// This method returns a `Vec<DevUuid>`. This `Vec` will contain UUIDs of
    /// devices that were newly unlocked while ignoring devices that are already
    /// in the unlocked state. If some devices are able to be unlocked
    /// and some fail, an error is returned as all devices should be able to
    /// be unlocked if the necessary key is in the keyring.
    async fn unlock_pool(
        &self,
        uuid: PoolUuid,
        unlock_method: UnlockMethod,
    ) -> StratisResult<SetUnlockAction<DevUuid>>;

    /// Find the pool designated by name or UUID.
    async fn get_pool(
        &self,
        key: LockKey<PoolUuid>,
    ) -> Option<SomeLockReadGuard<PoolUuid, Self::Pool>>;

    /// Get a mutable reference to the pool designated by name or UUID.
    async fn get_mut_pool(
        &self,
        key: LockKey<PoolUuid>,
    ) -> Option<SomeLockWriteGuard<PoolUuid, Self::Pool>>;

    /// Get a mapping of encrypted pool UUIDs for pools that have not yet
    /// been set up and need to be unlocked to their encryption infos.
    async fn locked_pools(&self) -> HashMap<PoolUuid, LockedPoolInfo>;

    /// Get all pools belonging to this engine.
    async fn pools(&self) -> AllLockReadGuard<PoolUuid, Self::Pool>;

    /// Get mutable references to all pools belonging to this engine.
    async fn pools_mut(&self) -> AllLockWriteGuard<PoolUuid, Self::Pool>;

    /// Get the UUIDs of all pools that experienced an event.
    async fn get_events(&self) -> StratisResult<HashSet<PoolUuid>>;

    /// Notify the engine that an event has occurred on the DM file descriptor
    /// and check pools for needed changes.
    async fn pool_evented(
        &self,
        pools: Option<&HashSet<PoolUuid>>,
    ) -> HashMap<PoolUuid, ThinPoolDiff>;

    /// Notify the engine that an event has occurred on the DM file descriptor
    /// and check filesystems for needed changes.
    async fn fs_evented(
        &self,
        pools: Option<&HashSet<PoolUuid>>,
    ) -> HashMap<FilesystemUuid, StratFilesystemDiff>;

    /// Get the handler for kernel keyring operations.
    async fn get_key_handler(&self) -> SharedGuard<OwnedRwLockReadGuard<Self::KeyActions>>;

    /// Get the handler for kernel keyring operations mutably.
    async fn get_key_handler_mut(&self) -> ExclusiveGuard<OwnedRwLockWriteGuard<Self::KeyActions>>;

    /// Return true if this engine is the simulator engine, otherwise false.
    fn is_sim(&self) -> bool;
}

/// Implements an interface for diffing two state structs.
pub trait StateDiff {
    type Diff;

    /// Run the diff and return what has changed. The newer state should always be
    /// the new_state argument as this method should always return the new values
    /// for any properties that are inconsistent.
    fn diff(&self, new_state: &Self) -> Self::Diff;
}

/// Dump all of the necessary state for the given data structure that may change.
pub trait DumpState {
    type State: StateDiff;

    /// Return a structure that can be diffed and contains all of the values that
    /// need to be checked in a diff and can change. This method should use
    /// existing cached stratisd data structures to determine the state.
    fn cached<F>(&self, f: F) -> Self::State
    where
        F: Fn(&Self) -> Self::State,
    {
        f(self)
    }

    /// Return a structure that can be diffed and contains all of the values that
    /// need to be checked in a diff and can change. This method should call
    /// out to fetch the current values of the state. A mutable reference is
    /// taken because this method should also update the cached values of the
    /// current state.
    fn dump<F>(&mut self, mut f: F) -> Self::State
    where
        F: FnMut(&mut Self) -> Self::State,
    {
        f(self)
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    os::unix::io::RawFd,
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use either::Either;
use serde_json::Value;

use devicemapper::{Bytes, Sectors};

use crate::{
    engine::{
        structures::{AllLockReadGuard, AllLockWriteGuard, SomeLockReadGuard, SomeLockWriteGuard},
        types::{
            ActionAvailability, BlockDevTier, Clevis, CreateAction, DeleteAction, DevUuid,
            EncryptedDevice, EncryptionInfo, FilesystemUuid, GrowAction, InputEncryptionInfo,
            IntegritySpec, Key, KeyDescription, LockedPoolsInfo, MappingCreateAction,
            MappingDeleteAction, Name, OptionalTokenSlotInput, PoolDiff, PoolEncryptionInfo,
            PoolIdentifier, PoolUuid, PropChangeAction, ReencryptedDevice, RegenAction,
            RenameAction, ReportType, SetCreateAction, SetDeleteAction, SetUnlockAction,
            StartAction, StopAction, StoppedPoolsInfo, StratBlockDevDiff, StratFilesystemDiff,
            StratSigblockVersion, TokenUnlockMethod, UdevEngineEvent, UnlockMethod,
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
    ///   with the appropriate key description and key data.
    /// * `Ok(MappingCreateAction::Created(_))`: The key was newly added to the
    ///   keyring.
    /// * `Ok(MappingCreateAction::Changed)`: The key description was already present
    ///   in the keyring but the key data was updated.
    fn set(
        &self,
        key_desc: &KeyDescription,
        key_fd: RawFd,
    ) -> StratisResult<MappingCreateAction<Key>>;

    /// Return a list of all key descriptions of keys added to the keyring by
    /// Stratis that are still valid.
    fn list(&self) -> StratisResult<Vec<KeyDescription>>;

    /// Unset a key with the given key description in the root persistent kernel
    /// keyring.
    fn unset(&self, key_desc: &KeyDescription) -> StratisResult<MappingDeleteAction<Key>>;
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

    /// Get filesystem size limit.
    fn size_limit(&self) -> Option<Sectors>;

    /// Get filesystem snapshot origin.
    fn origin(&self) -> Option<FilesystemUuid>;

    fn merge_scheduled(&self) -> bool;
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

    /// Get the newly registered size, if any, of the block device.
    ///
    /// If internally the new size is None, the block device size is equal to that
    /// registered in the BDA.
    fn new_size(&self) -> Option<Sectors>;

    /// Get metadata version from static header
    fn metadata_version(&self) -> StratSigblockVersion;
}

pub trait Pool: Debug + Send + Sync {
    /// Initialize the cache with the provided cache block devices.
    /// Returns a list of the the block devices that were actually added as cache
    /// devices. In practice, this will have three types of return values:
    /// * An error if the cache has already been initialized with a different set
    ///   of block devices.
    /// * `SetCreateAction::Identity` if the cache has already been initialized with
    ///   the same set of block devices.
    /// * `SetCreateAction::Created` containing all provided block devices if the
    ///   cache has not yet been initialized.
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
        supports_encrypted: bool,
    ) -> StratisResult<SetCreateAction<DevUuid>>;

    /// Creates the filesystems specified by specs.
    /// Returns a list of the names of filesystems actually created.
    /// Returns an error if any of the specified names are already in use
    /// for filesystems in this pool. If the same name is passed multiple
    /// times, the size associated with the last item is used.
    fn create_filesystems<'b>(
        &mut self,
        pool_name: &str,
        pool_uuid: PoolUuid,
        specs: &[(&'b str, Option<Bytes>, Option<Bytes>)],
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
    ) -> StratisResult<(SetCreateAction<DevUuid>, Option<PoolDiff>)>;

    /// V1: Binds all devices in the pool to a given Clevis config.
    /// * token_slot is always Legacy
    ///
    /// V2: Binds crypt device to a given Clevis config.
    /// * if token slot is Legacy bind with the assumption that there is only one Clevis token
    ///   slot allowed.
    /// * if token_slot is Some(_): bind to specific token slot
    /// * if token_slot is None: bind to any available token_slot
    fn bind_clevis(
        &mut self,
        token_slot: OptionalTokenSlotInput,
        pin: &str,
        clevis_info: &Value,
    ) -> StratisResult<CreateAction<(Clevis, u32)>>;

    /// V1: Binds all devices in the pool to a given key description.
    /// * token_slot is always None
    ///
    /// V2: Binds crypt device to a given key description.
    /// * if token slot is Legacy bind with the assumption that there is only one keyring token
    ///   slot allowed.
    /// * if token_slot is Some(_): bind to specific token slot
    /// * if token_slot is None: bind to any available token slot
    fn bind_keyring(
        &mut self,
        token_slot: OptionalTokenSlotInput,
        key_desc: &KeyDescription,
    ) -> StratisResult<CreateAction<(Key, u32)>>;

    /// V1: Rebinds all devices in the pool to a given key description.
    /// * token_slot is always None
    ///
    /// V2: Rebinds crypt device to a given key description.
    /// * if token_slot is Some(_): rebind specific token slot
    /// * if token_slot is None: rebind first keyring token slot
    fn rebind_keyring(
        &mut self,
        token_slot: Option<u32>,
        new_key_desc: &KeyDescription,
    ) -> StratisResult<RenameAction<Key>>;

    /// V1: Rebinds all devices in the pool to a given Clevis config.
    /// * token_slot is always None
    ///
    /// V2: Rebinds crypt device to a given Clevis config.
    /// * if token_slot is Some(_): rebind specific token slot
    /// * if token_slot is None: rebind first Clevis token slot
    fn rebind_clevis(&mut self, token_slot: Option<u32>) -> StratisResult<RegenAction>;

    /// V1: Unbinds crypt device from the key description token
    /// * token_slot is always None
    ///
    /// V2: Unbinds crypt device from the given key description token specified by token slot.
    /// * if token_slot is Some(_): unbind specific token slot
    /// * if token_slot is None: unbind first key description token slot
    fn unbind_keyring(&mut self, token_slot: Option<u32>) -> StratisResult<DeleteAction<Key>>;

    /// V1: Unbinds crypt device from the Clevis token
    /// * token_slot is always None
    ///
    /// V2: Unbinds crypt device from the given Clevis token specified by token slot.
    /// * if token_slot is Some(_): unbind specific token slot
    /// * if token_slot is None: unbind first Clevis token slot
    fn unbind_clevis(&mut self, token_slot: Option<u32>) -> StratisResult<DeleteAction<Clevis>>;

    /// Ensures that all designated filesystems are gone from pool.
    /// Returns a list of the filesystems found, and actually destroyed.
    /// This list will be a subset of the uuids passed in fs_uuids.
    /// Precondition: All filesystems given must be unmounted.
    fn destroy_filesystems(
        &mut self,
        pool_name: &str,
        fs_uuids: &HashSet<FilesystemUuid>,
    ) -> StratisResult<SetDeleteAction<FilesystemUuid, (FilesystemUuid, Option<FilesystemUuid>)>>;

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
    ) -> StratisResult<CreateAction<(FilesystemUuid, &mut dyn Filesystem)>>;

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
    /// by any subcomponent of Stratis, either for internal management or to
    /// store user data.
    fn total_physical_used(&self) -> Option<Sectors>;

    /// Get all the filesystems belonging to this pool.
    fn filesystems(&self) -> Vec<(Name, FilesystemUuid, &dyn Filesystem)>;

    /// Get the filesystem in this pool with this UUID.
    fn get_filesystem(&self, uuid: FilesystemUuid) -> Option<(Name, &dyn Filesystem)>;

    /// Get the filesystem in this pool with this name.
    fn get_filesystem_by_name(&self, name: &Name) -> Option<(FilesystemUuid, &dyn Filesystem)>;

    /// Get _all_ the blockdevs that belong to this pool.
    /// All really means all. For example, it does not exclude cache blockdevs.
    fn blockdevs(&self) -> Vec<(DevUuid, BlockDevTier, &dyn BlockDev)>;

    /// Get the blockdev in this pool with this UUID.
    fn get_blockdev(&self, uuid: DevUuid) -> Option<(BlockDevTier, &dyn BlockDev)>;

    /// Get the blockdev mutably in this pool with this UUID.
    fn get_mut_blockdev(
        &mut self,
        uuid: DevUuid,
    ) -> StratisResult<Option<(BlockDevTier, &mut dyn BlockDev)>>;

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

    /// Get all legacy encryption information for this pool.
    fn encryption_info_legacy(&self) -> Option<PoolEncryptionInfo>;

    /// Get all encryption information for this pool.
    fn encryption_info(&self) -> Option<Either<EncryptionInfo, PoolEncryptionInfo>>;

    /// Get the pool state for the given pool. The state indicates which actions
    /// will be disabled or enabled. Disabled actions are triggered by failures
    /// caught by stratisd.
    fn avail_actions(&self) -> ActionAvailability;

    /// Get the current limit on filesystems in this pool.
    fn fs_limit(&self) -> u64;

    /// Set the current limit on filesystems in this pool.
    fn set_fs_limit(
        &mut self,
        pool_name: &Name,
        pool_uuid: PoolUuid,
        new_limit: u64,
    ) -> StratisResult<()>;

    /// Check whether overprovisioning is disabled for the pool.
    fn overprov_enabled(&self) -> bool;

    /// Either enable or disable overprovisioning for the pool.
    fn set_overprov_mode(&mut self, pool_name: &Name, enabled: bool) -> StratisResult<()>;

    /// Returns a boolean indicating whether the pool is out of allocation space.
    fn out_of_alloc_space(&self) -> bool;

    /// Grow either a specified device or all devices in a pool if the underlying
    /// physical device or devices have changed in size.
    #[allow(clippy::type_complexity)]
    fn grow_physical(
        &mut self,
        pool_name: &Name,
        pool_uuid: PoolUuid,
        device: DevUuid,
    ) -> StratisResult<(GrowAction<(PoolUuid, DevUuid)>, Option<PoolDiff>)>;

    /// Set filesystem size limit.
    fn set_fs_size_limit(
        &mut self,
        fs: FilesystemUuid,
        limit: Option<Bytes>,
    ) -> StratisResult<PropChangeAction<Option<Sectors>>>;

    /// Encrypted an unencrypted pool.
    fn encrypt_pool(
        &mut self,
        name: &Name,
        pool_uuid: PoolUuid,
        encryption_info: &InputEncryptionInfo,
    ) -> StratisResult<CreateAction<EncryptedDevice>>;

    /// Reencrypt an encrypted pool.
    fn reencrypt_pool(&mut self) -> StratisResult<ReencryptedDevice>;

    /// Decrypt an encrypted pool.
    fn decrypt_pool(
        &mut self,
        name: &Name,
        pool_uuid: PoolUuid,
    ) -> StratisResult<DeleteAction<EncryptedDevice>>;

    /// Return the metadata that would be written if metadata were written.
    fn current_metadata(&self, pool_name: &Name) -> StratisResult<String>;

    /// Return the metadata that was last written to pool devices.
    fn last_metadata(&self) -> StratisResult<String>;

    /// Get the metadata version for a given pool.
    fn metadata_version(&self) -> StratSigblockVersion;

    /// Get the filesystem metadata that would be written if written now.
    fn current_fs_metadata(&self, fs_name: Option<&str>) -> StratisResult<String>;

    /// Get the last written filesystem metadata.
    fn last_fs_metadata(&self, fs_name: Option<&str>) -> StratisResult<String>;

    /// Set whether a merge of the filesystem is scheduled.
    fn set_fs_merge_scheduled(
        &mut self,
        fs: FilesystemUuid,
        new_scheduled: bool,
    ) -> StratisResult<PropChangeAction<bool>>;
}

pub type HandleEvents<P> = (
    Vec<SomeLockReadGuard<PoolUuid, P>>,
    HashMap<DevUuid, StratBlockDevDiff>,
);

#[async_trait]
pub trait Engine: Debug + Report + Send + Sync {
    /// Create a Stratis pool.
    /// Returns the UUID of the newly created pool.
    async fn create_pool(
        &self,
        name: &str,
        blockdev_paths: &[&Path],
        encryption_info: Option<&InputEncryptionInfo>,
        integrity_spec: IntegritySpec,
    ) -> StratisResult<CreateAction<PoolUuid>>;

    /// Handle a libudev event.
    /// If the handling action resulted in pool creation, return the pool
    /// and its UUID.
    ///
    /// Precondition: the subsystem of the device evented on is "block".
    async fn handle_events(&self, event: Vec<UdevEngineEvent>) -> HandleEvents<dyn Pool>;

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
        token_slot: UnlockMethod,
    ) -> StratisResult<SetUnlockAction<DevUuid>>;

    /// Find the pool designated by name or UUID.
    async fn get_pool(
        &self,
        key: PoolIdentifier<PoolUuid>,
    ) -> Option<SomeLockReadGuard<PoolUuid, dyn Pool>>;

    /// Get a mutable reference to the pool designated by name or UUID.
    async fn get_mut_pool(
        &self,
        key: PoolIdentifier<PoolUuid>,
    ) -> Option<SomeLockWriteGuard<PoolUuid, dyn Pool>>;

    /// Get a mapping of encrypted pool UUIDs for pools that have not yet
    /// been set up and need to be unlocked to their encryption infos.
    async fn locked_pools(&self) -> LockedPoolsInfo;

    /// Get a mapping of pool UUIDs for pools that have not yet
    /// been set up and need to be started to their device infos.
    async fn stopped_pools(&self) -> StoppedPoolsInfo;

    /// Get all pools belonging to this engine.
    async fn pools(&self) -> AllLockReadGuard<PoolUuid, dyn Pool>;

    /// Get mutable references to all pools belonging to this engine.
    async fn pools_mut(&self) -> AllLockWriteGuard<PoolUuid, dyn Pool>;

    /// Get the UUIDs of all pools that experienced an event.
    async fn get_events(&self) -> StratisResult<HashSet<PoolUuid>>;

    /// Notify the engine that an event has occurred on the DM file descriptor
    /// and check pools for needed changes.
    async fn pool_evented(&self, pools: Option<&HashSet<PoolUuid>>) -> HashMap<PoolUuid, PoolDiff>;

    /// Notify the engine that an event has occurred on the DM file descriptor
    /// and check filesystems for needed changes.
    async fn fs_evented(
        &self,
        pools: Option<&HashSet<PoolUuid>>,
    ) -> HashMap<FilesystemUuid, StratFilesystemDiff>;

    /// Get the handler for kernel keyring operations.
    async fn get_key_handler(&self) -> Arc<dyn KeyActions>;

    /// Start and set up a pool, creating all necessary devicemapper devices to
    /// perform IO operations and start monitoring for events.
    async fn start_pool(
        &self,
        pool_id: PoolIdentifier<PoolUuid>,
        token_slot: TokenUnlockMethod,
        passphrase_fd: Option<RawFd>,
    ) -> StratisResult<StartAction<PoolUuid>>;

    /// Stop and tear down a pool, storing the information for it to be started
    /// again later.
    async fn stop_pool(
        &self,
        pool_id: PoolIdentifier<PoolUuid>,
        has_partially_constructed: bool,
    ) -> StratisResult<StopAction<PoolUuid>>;

    /// Refresh the state of all pools and liminal devices.
    async fn refresh_state(&self) -> StratisResult<()>;

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

    /// Return all contained values unchanged.
    fn unchanged(&self) -> Self::Diff;
}

/// Dump all of the necessary state for the given data structure that may change.
pub trait DumpState<'a> {
    type State: StateDiff;
    type DumpInput;

    /// Return a structure that can be diffed and contains all of the values that
    /// need to be checked in a diff and can change. This method should use
    /// existing cached stratisd data structures to determine the state.
    fn cached(&self) -> Self::State;

    /// Return a structure that can be diffed and contains all of the values that
    /// need to be checked in a diff and can change. This method should call
    /// out to fetch the current values of the state. A mutable reference is
    /// taken because this method should also update the cached values of the
    /// current state.
    fn dump(&mut self, input: Self::DumpInput) -> Self::State;
}

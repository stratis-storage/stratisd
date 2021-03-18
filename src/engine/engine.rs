// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::HashMap,
    fmt::Debug,
    os::unix::io::RawFd,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde_json::Value;

use devicemapper::{Bytes, Sectors};

use crate::{
    engine::types::{
        BlockDevTier, Clevis, CreateAction, DeleteAction, DevUuid, EncryptionInfo, FilesystemUuid,
        Key, KeyDescription, LockedPoolInfo, MappingCreateAction, Name, PoolUuid, RenameAction,
        ReportType, SetCreateAction, SetDeleteAction, SetUnlockAction, UdevEngineEvent,
        UnlockMethod,
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
    fn unset(&mut self, key_desc: &KeyDescription) -> StratisResult<DeleteAction<Key>>;
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
    /// Send a synthetic udev change event to the devicemapper device representing
    /// the filesystem.
    fn send_udev_change(
        &self,
        pool_name: &str,
        fs_uuid: FilesystemUuid,
        fs_name: &str,
    ) -> StratisResult<()>;

    /// path of the device node
    fn devnode(&self) -> PathBuf;

    /// When the filesystem was created.
    fn created(&self) -> DateTime<Utc>;

    /// path to mount the filesystem by
    fn path_to_mount_filesystem(&self, pool_name: &str, fs_name: &str) -> PathBuf;

    /// The amount of data stored on the filesystem, including overhead.
    fn used(&self) -> StratisResult<Bytes>;
}

pub trait BlockDev: Debug {
    /// Get the device path for the block device.
    fn devnode(&self) -> &Path;

    /// Get the canonicalized path to display to the user representing this block device.
    fn user_path(&self) -> StratisResult<PathBuf>;

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

pub trait Pool: Debug {
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
        pool_uuid: PoolUuid,
        specs: &[(&'b str, Option<Sectors>)],
    ) -> StratisResult<SetCreateAction<(&'b str, FilesystemUuid)>>;

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
        pin: String,
        clevis_info: Value,
    ) -> StratisResult<CreateAction<Clevis>>;

    /// Unbind all devices in the given pool from using clevis.
    fn unbind_clevis(&mut self) -> StratisResult<DeleteAction<Clevis>>;

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

    /// The number of Sectors in this pool that are currently in use by the
    /// pool for some purpose, and therefore not available for future use,
    /// by any subcomponent of Stratis, either for internal managment or to
    /// store user data.
    fn total_physical_used(&self) -> StratisResult<Sectors>;

    /// Get all the filesystems belonging to this pool.
    fn filesystems(&self) -> Vec<(Name, FilesystemUuid, &dyn Filesystem)>;

    /// Get all the filesystems belonging to this pool as mutable references.
    fn filesystems_mut(&mut self) -> Vec<(Name, FilesystemUuid, &mut dyn Filesystem)>;

    /// Get the filesystem in this pool with this UUID.
    fn get_filesystem(&self, uuid: FilesystemUuid) -> Option<(Name, &dyn Filesystem)>;

    /// Get the mutable filesystem in this pool with this UUID.
    fn get_mut_filesystem(&mut self, uuid: FilesystemUuid) -> Option<(Name, &mut dyn Filesystem)>;

    /// Get _all_ the blockdevs that belong to this pool.
    /// All really means all. For example, it does not exclude cache blockdevs.
    fn blockdevs(&self) -> Vec<(DevUuid, BlockDevTier, &dyn BlockDev)>;

    /// Get all the blockdevs belonging to this pool as mutable references.
    fn blockdevs_mut(&mut self) -> Vec<(DevUuid, BlockDevTier, &mut dyn BlockDev)>;

    /// Get the blockdev in this pool with this UUID.
    fn get_blockdev(&self, uuid: DevUuid) -> Option<(BlockDevTier, &dyn BlockDev)>;

    /// Get a mutable reference to the blockdev in this pool with this UUID.
    fn get_mut_blockdev(&mut self, uuid: DevUuid) -> Option<(BlockDevTier, &mut dyn BlockDev)>;

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
    fn encryption_info(&self) -> Option<&EncryptionInfo>;
}

pub trait Engine: Debug + Report + Send {
    /// Create a Stratis pool.
    /// Returns the UUID of the newly created pool.
    /// Returns an error if the redundancy code does not correspond to a
    /// supported redundancy.
    fn create_pool(
        &mut self,
        name: &str,
        blockdev_paths: &[&Path],
        redundancy: Option<u16>,
        key_desc: Option<KeyDescription>,
    ) -> StratisResult<CreateAction<PoolUuid>>;

    /// Handle a libudev event.
    /// If the handling action resulted in pool creation, return the pool
    /// and its UUID.
    ///
    /// Precondition: the subsystem of the device evented on is "block".
    fn handle_event(&mut self, event: &UdevEngineEvent) -> Option<(Name, PoolUuid, &dyn Pool)>;

    /// Destroy a pool.
    /// Ensures that the pool of the given UUID is absent on completion.
    /// Returns true if some action was necessary, otherwise false.
    fn destroy_pool(&mut self, uuid: PoolUuid) -> StratisResult<DeleteAction<PoolUuid>>;

    /// Rename pool with uuid to new_name.
    /// Raises an error if the mapping can't be applied because
    /// new_name is already in use.
    /// Returns true if it was necessary to perform an action, false if not.
    fn rename_pool(
        &mut self,
        uuid: PoolUuid,
        new_name: &str,
    ) -> StratisResult<RenameAction<PoolUuid>>;

    /// Unlock all encrypted devices registered under a given pool UUID.
    /// This method returns a `Vec<DevUuid>`. This `Vec` will contain UUIDs of
    /// devices that were newly unlocked while ignoring devices that are already
    /// in the unlocked state. If some devices are able to be unlocked
    /// and some fail, an error is returned as all devices should be able to
    /// be unlocked if the necessary key is in the keyring.
    fn unlock_pool(
        &mut self,
        uuid: PoolUuid,
        unlock_method: UnlockMethod,
    ) -> StratisResult<SetUnlockAction<DevUuid>>;

    /// Find the pool designated by uuid.
    fn get_pool(&self, uuid: PoolUuid) -> Option<(Name, &dyn Pool)>;

    /// Get a mutable referent to the pool designated by uuid.
    fn get_mut_pool(&mut self, uuid: PoolUuid) -> Option<(Name, &mut dyn Pool)>;

    /// Get a mapping of encrypted pool UUIDs for pools that have not yet
    /// been set up and need to be unlocked to their encryption infos.
    fn locked_pools(&self) -> HashMap<PoolUuid, LockedPoolInfo>;

    /// Configure the simulator. Obsolete.
    fn configure_simulator(&mut self, _: u32) -> StratisResult<()> {
        Ok(())
    }

    /// Get all pools belonging to this engine.
    fn pools(&self) -> Vec<(Name, PoolUuid, &dyn Pool)>;

    /// Get mutable references to all pools belonging to this engine.
    fn pools_mut(&mut self) -> Vec<(Name, PoolUuid, &mut dyn Pool)>;

    /// Notify the engine that an event has occurred on the DM file descriptor.
    fn evented(&mut self) -> StratisResult<()>;

    /// Get the handler for kernel keyring operations.
    fn get_key_handler(&self) -> &dyn KeyActions;

    /// Get the handler for kernel keyring operations mutably.
    fn get_key_handler_mut(&mut self) -> &mut dyn KeyActions;

    /// Return true if this engine is the simulator engine, otherwise false.
    fn is_sim(&self) -> bool;
}

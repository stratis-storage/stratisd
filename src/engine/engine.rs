// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fmt::Debug;
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use devicemapper::{Bytes, Device, Sectors};

use super::types::{
    BlockDevState, BlockDevTier, DevUuid, FilesystemUuid, MaybeDbusPath, Name, PoolState, PoolUuid,
    RenameAction,
};
use stratis::StratisResult;

pub const DEV_PATH: &str = "/dev/stratis";

pub trait Filesystem: Debug {
    /// path of the device node
    fn devnode(&self) -> PathBuf;

    /// When the filesystem was created.
    fn created(&self) -> DateTime<Utc>;

    /// The amount of data stored on the filesystem, including overhead.
    fn used(&self) -> StratisResult<Bytes>;

    /// Set dbus path associated with the Pool.
    fn set_dbus_path(&mut self, path: MaybeDbusPath) -> ();

    /// Get dbus path associated with the Pool.
    fn get_dbus_path(&self) -> &MaybeDbusPath;
}

pub trait BlockDev: Debug {
    /// Get the path of the device node for this device.
    fn devnode(&self) -> PathBuf;

    /// Get the user-settable string associated with this blockdev.
    fn user_info(&self) -> Option<&str>;

    /// Get the hardware ID for this blockdev.
    fn hardware_info(&self) -> Option<&str>;

    /// The time that this blockdev was initialized by Stratis, rounded down
    /// to the nearest second.
    fn initialization_time(&self) -> DateTime<Utc>;

    /// The total size of the device, including space not usable for data.
    fn size(&self) -> Sectors;

    /// The current state of the blockdev.
    fn state(&self) -> BlockDevState;

    /// Set dbus path associated with the BlockDev.
    fn set_dbus_path(&mut self, path: MaybeDbusPath) -> ();

    /// Get dbus path associated with the BlockDev.
    fn get_dbus_path(&self) -> &MaybeDbusPath;
}

pub trait Pool: Debug {
    /// Creates the filesystems specified by specs.
    /// Returns a list of the names of filesystems actually created.
    /// Returns an error if any of the specified names are already in use
    /// for filesystems in this pool. If the same name is passed multiple
    /// times, the size associated with the last item is used.
    fn create_filesystems<'a, 'b>(
        &'a mut self,
        pool_uuid: PoolUuid,
        pool_name: &str,
        specs: &[(&'b str, Option<Sectors>)],
    ) -> StratisResult<Vec<(&'b str, FilesystemUuid)>>;

    /// Adds blockdevs specified by paths to pool.
    /// Returns a list of uuids corresponding to devices actually added.
    /// Returns an error if a blockdev can not be added because it is owned
    /// or there was an error while reading or writing a blockdev.
    fn add_blockdevs(
        &mut self,
        pool_uuid: PoolUuid,
        pool_name: &str,
        paths: &[&Path],
        tier: BlockDevTier,
        force: bool,
    ) -> StratisResult<Vec<DevUuid>>;

    /// Destroy the pool.
    /// Precondition: All filesystems belonging to this pool must be
    /// unmounted.
    fn destroy(&mut self) -> StratisResult<()>;

    /// Ensures that all designated filesystems are gone from pool.
    /// Returns a list of the filesystems found, and actually destroyed.
    /// This list will be a subset of the uuids passed in fs_uuids.
    /// Precondition: All filesystems given must be unmounted.
    fn destroy_filesystems<'a>(
        &'a mut self,
        pool_name: &str,
        fs_uuids: &[FilesystemUuid],
    ) -> StratisResult<Vec<FilesystemUuid>>;

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
    ) -> StratisResult<RenameAction>;

    /// Snapshot filesystem
    /// Create a CoW snapshot of the origin
    fn snapshot_filesystem(
        &mut self,
        pool_uuid: PoolUuid,
        pool_name: &str,
        origin_uuid: FilesystemUuid,
        snapshot_name: &str,
    ) -> StratisResult<(FilesystemUuid, &mut Filesystem)>;

    /// The total number of Sectors belonging to this pool.
    /// There are no exclusions, so this number includes overhead sectors
    /// of all sorts, sectors allocated for every sort of metadata by
    /// Stratis or devicemapper and therefore not available to the user for
    /// storing their data. There is no larger physical size number that can be
    /// associated with a pool.
    fn total_physical_size(&self) -> Sectors;

    /// The number of Sectors in this pool that are currently in use by the
    /// pool for some purpose, be it to store metadata, to store user data,
    /// or to reserve for some other purpose.
    fn total_physical_used(&self) -> StratisResult<Sectors>;

    /// Get all the filesystems belonging to this pool.
    fn filesystems(&self) -> Vec<(Name, FilesystemUuid, &Filesystem)>;

    /// Get all the filesystems belonging to this pool as mutable references.
    fn filesystems_mut(&mut self) -> Vec<(Name, FilesystemUuid, &mut Filesystem)>;

    /// Get the filesystem in this pool with this UUID.
    fn get_filesystem(&self, uuid: FilesystemUuid) -> Option<(Name, &Filesystem)>;

    /// Get the mutable filesystem in this pool with this UUID.
    fn get_mut_filesystem(&mut self, uuid: FilesystemUuid) -> Option<(Name, &mut Filesystem)>;

    /// Get _all_ the blockdevs that belong to this pool.
    /// All really means all. For example, it does not exclude cache blockdevs.
    fn blockdevs(&self) -> Vec<(Uuid, &BlockDev)>;

    /// Get all the blockdevs belonging to this pool as mutable references.
    fn blockdevs_mut(&mut self) -> Vec<(DevUuid, &mut BlockDev)>;

    /// Get the blockdev in this pool with this UUID.
    fn get_blockdev(&self, uuid: DevUuid) -> Option<(BlockDevTier, &BlockDev)>;

    /// Get a mutable reference to the blockdev in this pool with this UUID.
    fn get_mut_blockdev(&mut self, uuid: DevUuid) -> Option<(BlockDevTier, &mut BlockDev)>;

    /// Set the user-settable string associated with the blockdev specified
    /// by the uuid.
    fn set_blockdev_user_info(
        &mut self,
        pool_name: &str,
        uuid: DevUuid,
        user_info: Option<&str>,
    ) -> StratisResult<bool>;

    /// The current state of the Pool.
    fn state(&self) -> PoolState;

    /// Set dbus path associated with the Pool.
    fn set_dbus_path(&mut self, path: MaybeDbusPath) -> ();

    /// Get dbus path associated with the Pool.
    fn get_dbus_path(&self) -> &MaybeDbusPath;
}

pub trait Engine: Debug {
    /// Create a Stratis pool.
    /// Returns the UUID of the newly created pool.
    /// Returns an error if the redundancy code does not correspond to a
    /// supported redundancy.
    fn create_pool(
        &mut self,
        name: &str,
        blockdev_paths: &[&Path],
        redundancy: Option<u16>,
        force: bool,
    ) -> StratisResult<PoolUuid>;

    /// Evaluate a device node & devicemapper::Device to see if it's a valid
    /// stratis device.  If all the devices are present in the pool and the pool isn't already
    /// up and running, it will get setup and the pool uuid will be returned.
    fn block_evaluate(
        &mut self,
        device: Device,
        dev_node: PathBuf,
    ) -> StratisResult<Option<PoolUuid>>;

    /// Destroy a pool.
    /// Ensures that the pool of the given UUID is absent on completion.
    /// Returns true if some action was necessary, otherwise false.
    fn destroy_pool(&mut self, uuid: PoolUuid) -> StratisResult<bool>;

    /// Rename pool with uuid to new_name.
    /// Raises an error if the mapping can't be applied because
    /// new_name is already in use.
    /// Returns true if it was necessary to perform an action, false if not.
    fn rename_pool(&mut self, uuid: PoolUuid, new_name: &str) -> StratisResult<RenameAction>;

    /// Find the pool designated by uuid.
    fn get_pool(&self, uuid: PoolUuid) -> Option<(Name, &Pool)>;

    /// Get a mutable referent to the pool designated by uuid.
    fn get_mut_pool(&mut self, uuid: PoolUuid) -> Option<(Name, &mut Pool)>;

    /// Configure the simulator, for the real engine, this is a null op.
    /// denominator: the probably of failure is 1/denominator.
    fn configure_simulator(&mut self, denominator: u32) -> StratisResult<()>;

    /// Get all pools belonging to this engine.
    fn pools(&self) -> Vec<(Name, PoolUuid, &Pool)>;

    /// Get mutable references to all pools belonging to this engine.
    fn pools_mut(&mut self) -> Vec<(Name, PoolUuid, &mut Pool)>;

    /// If the engine would like to include an event in the message loop, it
    /// may return an Eventable from this method.
    fn get_eventable(&self) -> Option<&'static Eventable>;

    /// Notify the engine that an event has occurred on the Eventable.
    fn evented(&mut self) -> StratisResult<()>;
}

/// Allows an Engine to include a fd in the event loop. See
/// Engine::get_eventable() and Engine::evented().
pub trait Eventable {
    /// Get fd the engine would like to monitor for activity
    fn get_pollable_fd(&self) -> RawFd;

    /// Assuming level-triggered semantics, clear the event that caused the
    /// Eventable to trigger.
    fn clear_event(&self) -> StratisResult<()>;
}

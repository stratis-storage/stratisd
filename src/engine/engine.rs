// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fmt::Debug;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use devicemapper::Sectors;

use super::errors::EngineResult;
use super::types::{FilesystemUuid, PoolUuid, RenameAction};

pub trait HasUuid: Debug {
    fn uuid(&self) -> &Uuid;
}

pub trait HasName: Debug {
    fn name(&self) -> &str;
}

pub trait Dev: Debug {}

pub trait Filesystem: HasName + HasUuid {
    /// path of the device node
    fn devnode(&self) -> EngineResult<PathBuf>;
}

pub trait Pool: HasName + HasUuid {
    /// Creates the filesystems specified by specs.
    /// Returns a list of the names of filesystems actually created.
    /// Returns an error if any of the specified names are already in use
    /// for filesystems in this pool. If the same name is passed multiple
    /// times, the size associated with the last item is used.
    fn create_filesystems<'a, 'b>(&'a mut self,
                                  specs: &[(&'b str, Option<Sectors>)])
                                  -> EngineResult<Vec<(&'b str, FilesystemUuid)>>;

    /// Adds blockdevs specified by paths to pool.
    /// Returns a list of device nodes corresponding to devices actually added.
    /// Returns an error if a blockdev can not be added because it is owned
    /// or there was an error while reading or writing a blockdev.
    fn add_blockdevs(&mut self, paths: &[&Path], force: bool) -> EngineResult<Vec<PathBuf>>;

    /// Destroy the pool.
    /// Precondition: All filesystems belonging to this pool must be
    /// unmounted.
    fn destroy(self) -> EngineResult<()>;

    /// Ensures that all designated filesystems are gone from pool.
    /// Returns a list of the filesystems found, and actually destroyed.
    /// This list will be a subset of the uuids passed in fs_uuids.
    /// Precondition: All filesystems given must be unmounted.
    fn destroy_filesystems<'a, 'b>(&'a mut self,
                                   fs_uuids: &[&'b FilesystemUuid])
                                   -> EngineResult<Vec<&'b FilesystemUuid>>;

    /// Rename filesystem
    /// Rename pool with uuid to new_name.
    /// Raises an error if the mapping can't be applied because
    /// new_name is already in use.
    /// The result indicate whether an action was performed, and if not, why.
    fn rename_filesystem(&mut self,
                         uuid: &FilesystemUuid,
                         new_name: &str)
                         -> EngineResult<RenameAction>;

    /// Rename this pool.
    fn rename(&mut self, name: &str) -> ();

    /// Get the filesystem in this pool with this UUID.
    fn get_filesystem(&self, uuid: &FilesystemUuid) -> Option<&Filesystem>;

    /// Get the mutable filesystem in this pool with this UUID.
    fn get_mut_filesystem(&mut self, uuid: &FilesystemUuid) -> Option<&mut Filesystem>;

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
    fn total_physical_used(&self) -> EngineResult<Sectors>;

    /// Get all the filesystems belonging to this pool.
    fn filesystems(&self) -> Vec<&Filesystem>;
}

pub trait Engine: Debug {
    /// Create a Stratis pool.
    /// Returns the UUID of the newly created pool and the blockdevs the
    /// pool contains.
    /// Returns an error if the redundancy code does not correspond to a
    /// supported redundancy.
    fn create_pool(&mut self,
                   name: &str,
                   blockdev_paths: &[&Path],
                   redundancy: Option<u16>,
                   force: bool)
                   -> EngineResult<(PoolUuid, Vec<PathBuf>)>;

    /// Destroy a pool.
    /// Ensures that the pool of the given UUID is absent on completion.
    /// Returns true if some action was necessary, otherwise false.
    fn destroy_pool(&mut self, uuid: &PoolUuid) -> EngineResult<bool>;

    /// Rename pool with uuid to new_name.
    /// Raises an error if the mapping can't be applied because
    /// new_name is already in use.
    /// Returns true if it was necessary to perform an action, false if not.
    fn rename_pool(&mut self, uuid: &PoolUuid, new_name: &str) -> EngineResult<RenameAction>;

    /// Find the pool designated by uuid.
    fn get_pool(&self, uuid: &PoolUuid) -> Option<&Pool>;

    /// Get a mutable referent to the pool designated by uuid.
    fn get_mut_pool(&mut self, uuid: &PoolUuid) -> Option<&mut Pool>;

    /// Configure the simulator, for the real engine, this is a null op.
    /// denominator: the probably of failure is 1/denominator.
    fn configure_simulator(&mut self, denominator: u32) -> EngineResult<()>;

    /// Check pools' current state and take appropriate actions
    fn check(&mut self) -> ();

    /// Get all pools belonging to this engine.
    fn pools(&self) -> Vec<&Pool>;
}

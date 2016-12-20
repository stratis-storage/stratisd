// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use nix;

#[derive(Debug)]
pub enum RenameAction {
    Identity,
    NoSource,
    Renamed,
}

#[derive(Debug)]
pub enum ErrorEnum {
    Ok,
    Error(String),

    AlreadyExists(String),
    Busy(String),
    Invalid(String),
    NotFound(String),
}

impl ErrorEnum {
    pub fn get_error_string(&self) -> String {
        match *self {
            ErrorEnum::Ok => "Ok".into(),
            ErrorEnum::Error(ref x) => format!("{}", x),
            ErrorEnum::AlreadyExists(ref x) => format!("{} already exists", x),
            ErrorEnum::Busy(ref x) => format!("{} is busy", x),
            ErrorEnum::Invalid(ref x) => format!("{}", x),
            ErrorEnum::NotFound(ref x) => format!("{} is not found", x),
        }
    }
}

#[derive(Debug)]
pub enum EngineError {
    Stratis(ErrorEnum),
    Io(io::Error),
    Nix(nix::Error),
}

pub type EngineResult<T> = Result<T, EngineError>;

pub trait Dev: Debug {
    fn get_id(&self) -> String;
}

pub trait Cache: Debug {
    fn get_id(&self) -> String;
}

pub trait Filesystem: Debug {}

impl From<io::Error> for EngineError {
    fn from(err: io::Error) -> EngineError {
        EngineError::Io(err)
    }
}

impl From<nix::Error> for EngineError {
    fn from(err: nix::Error) -> EngineError {
        EngineError::Nix(err)
    }
}

pub trait Pool: Debug {
    /// Creates the filesystems specified by specs.
    /// Returns a list of the names of filesystems actually created.
    /// Returns an error if any of the specified names are already in use
    /// for filesystems in this pool.
    fn create_filesystems<'a, 'b, 'c>(&'a mut self,
                                      specs: &[(&'b str, &'c str, Option<u64>)])
                                      -> EngineResult<Vec<&'b str>>;

    fn create_snapshot<'a, 'b, 'c>(&'a mut self,
                                   snapshot_name: &'b str,
                                   source: &'c str)
                                   -> EngineResult<&'b str>;

    /// Adds blockdevs specified by paths to pool.
    /// Returns a list of device nodes corresponding to devices actually added.
    /// Returns an error if a blockdev can not be added because it is owned
    /// or there was an error while reading or writing a blockdev.
    fn add_blockdevs(&mut self, paths: &[&Path], force: bool) -> EngineResult<Vec<PathBuf>>;


    /// Adds blockdevs specified by paths to pool cache.
    /// Returns a list of device nodes corresponding to devices actually added.
    /// Returns an error if a blockdev can not be added because it is owned
    /// or there was an error while reading or writing a blockdev.
    fn add_cachedevs(&mut self, paths: &[&Path], force: bool) -> EngineResult<Vec<PathBuf>>;

    /// Remove specified block devices from this pool.
    /// Returns a list of the devices actually removed.
    /// Returns an error if a device could not be removed.
    fn remove_blockdevs(&mut self, path: &[&Path]) -> EngineResult<Vec<PathBuf>>;

    /// Remove specified cache devices from this pool.
    /// Returns a list of the devices actually removed.
    /// Returns an error if a device could not be removed.
    fn remove_cachedevs(&mut self, path: &[&Path]) -> EngineResult<Vec<PathBuf>>;

    fn filesystems(&mut self) -> BTreeMap<&str, &mut Filesystem>;
    fn blockdevs(&mut self) -> Vec<&mut Dev>;
    fn cachedevs(&mut self) -> Vec<&mut Cache>;

    /// Ensures that all designated filesystems are gone from pool.
    /// Returns a list of the filesystems found, and actually destroyed.
    /// This list will be a subset of the names passed in fs_names.
    fn destroy_filesystems<'a, 'b>(&'a mut self,
                                   fs_names: &[&'b str])
                                   -> EngineResult<Vec<&'b str>>;

    /// Rename filesystem
    /// Applies a mapping from old name to new name.
    /// Raises an error if the mapping can't be applied because
    /// the names aren't equal and both are in use.
    /// The result indicate whether an action was performed, and if not, why.
    fn rename_filesystem(&mut self, old_name: &str, new_name: &str) -> EngineResult<RenameAction>;
}

pub trait Engine: Debug {
    /// Create a Stratis pool. Returns the number of blockdevs the pool contains.
    fn create_pool(&mut self,
                   name: &str,
                   blockdev_paths: &[&Path],
                   raid_level: u16,
                   force: bool)
                   -> EngineResult<Vec<PathBuf>>;

    /// Destroy a pool.
    /// Ensures that the pool of the given name is absent on completion.
    /// Returns true if some action was necessary, otherwise false.
    fn destroy_pool(&mut self, name: &str) -> EngineResult<bool>;

    /// Rename pool
    /// Applies a mapping from old name to new name.
    /// Raises an error if the mapping can't be applied because
    /// the names aren't equal and both are in use.
    /// Returns true if it was necessary to perform an action, false if not.
    fn rename_pool(&mut self, old_name: &str, new_name: &str) -> EngineResult<RenameAction>;

    fn get_pool(&mut self, name: &str) -> EngineResult<&mut Pool>;
    fn pools(&mut self) -> BTreeMap<&str, &mut Pool>;

    /// Configure the simulator, for the real engine, this is a null op.
    /// denominator: the probably of failure is 1/denominator.
    fn configure_simulator(&mut self, denominator: u32) -> EngineResult<()>;
}

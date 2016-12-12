// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use nix;

use uuid::Uuid;

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
    fn has_same(&self, other: &Path) -> bool;
}

pub trait Cache: Debug {
    fn get_id(&self) -> String;
    fn has_same(&self, other: &Path) -> bool;
}

pub trait Filesystem: Debug {
    fn get_id(&self) -> Uuid;
    fn eq(&self, other: &Filesystem) -> bool;
    fn get_name(&self) -> String;
    fn has_same(&self, other: &str) -> bool;
    fn rename(&mut self, new_name: &str) -> EngineResult<()>;
    fn add_ancestor(&mut self, parent: Uuid);
}

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
    fn create_filesystem(&mut self,
                         name: &str,
                         mount_point: &str,
                         quota_size: Option<u64>)
                         -> EngineResult<Uuid>;
    fn create_snapshot(&mut self, snapshot_name: &str, source: &str) -> EngineResult<()>;
    fn add_blockdevs(&mut self, paths: &[&Path], force: bool) -> EngineResult<Vec<PathBuf>>;
    fn add_cachedevs(&mut self, paths: &[&Path], force: bool) -> EngineResult<Vec<PathBuf>>;
    fn remove_blockdev(&mut self, path: &Path) -> EngineResult<()>;
    fn remove_cachedev(&mut self, path: &Path) -> EngineResult<()>;
    fn filesystems(&mut self) -> BTreeMap<&Uuid, &mut Filesystem>;
    fn blockdevs(&mut self) -> Vec<&mut Dev>;
    fn cachedevs(&mut self) -> Vec<&mut Cache>;
    fn destroy_filesystem(&mut self, name: &str) -> EngineResult<()>;
    fn get_filesystem_by_id(&mut self, id: &Uuid) -> EngineResult<&mut Filesystem>;
    fn get_filesystem_id(&self, name: &str) -> EngineResult<Uuid>;
    fn get_filesystem_by_name(&mut self, name: &str) -> EngineResult<&mut Filesystem>;
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

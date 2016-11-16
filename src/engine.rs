// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io;
use std::path::Path;

use nix;

use uuid::Uuid;

#[derive(Debug)]
pub enum ErrorEnum {
    Ok,
    Error(String),

    AlreadyExists(String),
    Busy(String),
    NotFound(String),
}

impl ErrorEnum {
    pub fn get_error_string(&self) -> String {
        match *self {
            ErrorEnum::Ok => "Ok".into(),
            ErrorEnum::Error(ref x) => format!("{}", x),
            ErrorEnum::AlreadyExists(ref x) => format!("{} already exists", x),
            ErrorEnum::Busy(ref x) => format!("{} is busy", x),
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
    fn create_filesystem(&mut self, name: &str, mount_point: &str, size: u64) -> EngineResult<()>;
    fn add_blockdev(&mut self, path: &Path) -> EngineResult<()>;
    fn add_cachedev(&mut self, path: &Path) -> EngineResult<()>;
    fn remove_blockdev(&mut self, path: &Path) -> EngineResult<()>;
    fn remove_cachedev(&mut self, path: &Path) -> EngineResult<()>;
    fn filesystems(&mut self) -> BTreeMap<&Uuid, &mut Filesystem>;
    fn blockdevs(&mut self) -> Vec<&mut Dev>;
    fn cachedevs(&mut self) -> Vec<&mut Cache>;
    fn destroy_filesystem(&mut self, name: &str) -> EngineResult<()>;
    fn get_filesystem(&mut self, id: &Uuid) -> EngineResult<&mut Filesystem>;
    fn get_filesystem_id(&mut self, name: &str) -> EngineResult<Uuid>;
}

pub trait Engine: Debug {
    fn create_pool(&mut self,
                   name: &str,
                   blockdev_paths: &[&Path],
                   raid_level: u16)
                   -> EngineResult<()>;
    fn destroy_pool(&mut self, name: &str) -> EngineResult<()>;
    fn get_pool(&mut self, name: &str) -> EngineResult<&mut Pool>;
    fn pools(&mut self) -> BTreeMap<&str, &mut Pool>;

    /// Configure the simulator, for the real engine, this is a null op.
    /// denominator: the probably of failure is 1/denominator.
    fn configure_simulator(&mut self, denominator: u32) -> EngineResult<()>;
}

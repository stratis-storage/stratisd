// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.


use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io;
use std::path::Path;

use nix;

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
    fn copy(&self) -> Box<Dev>;
    fn get_id(&self) -> String;
    fn has_same(&self, other: &Path) -> bool;
}

pub trait Cache: Debug {
    fn copy(&self) -> Box<Cache>;
    fn get_id(&self) -> String;
    fn has_same(&self, other: &Path) -> bool;
}

pub trait Filesystem: Debug {
    fn copy(&self) -> Box<Filesystem>;
    fn get_id(&self) -> String;
    fn eq(&self, other: &Filesystem) -> bool;
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
                         filesystem_name: &str,
                         mount_point: &str,
                         size: u64)
                         -> EngineResult<()>;
    fn add_blockdev(&mut self, path: &Path) -> EngineResult<()>;
    fn add_cachedev(&mut self, path: &Path) -> EngineResult<()>;
    fn remove_blockdev(&mut self, path: &Path) -> EngineResult<()>;
    fn remove_cachedev(&mut self, path: &Path) -> EngineResult<()>;
    fn destroy(&mut self) -> EngineResult<()>;
    fn list_filesystems(&self) -> EngineResult<BTreeMap<String, Box<Filesystem>>>;
    fn list_blockdevs(&self) -> EngineResult<Vec<Box<Dev>>>;
    fn list_cachedevs(&self) -> EngineResult<Vec<Box<Cache>>>;
    fn copy(&self) -> Box<Pool>;
}

pub trait Engine: Debug {
    fn create_pool(&mut self,
                   name: &str,
                   blockdev_paths: &[&Path],
                   raid_level: u16)
                   -> EngineResult<()>;
    fn destroy_pool(&mut self, name: &str) -> EngineResult<()>;
    fn get_pool(&mut self, name: &str) -> EngineResult<&mut Box<Pool>>;
    fn list_pools(&self) -> EngineResult<BTreeMap<String, Box<Pool>>>;

    /// Configure the simulator, for the real engine, this is a null op.
    /// denominator: the probably of failure is 1/denominator.
    fn configure_simulator(&mut self, denominator: u32) -> EngineResult<()>;
}

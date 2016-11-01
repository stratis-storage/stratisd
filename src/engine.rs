// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use bidir_map::BidirMap;

use core::cmp::PartialEq;

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

pub trait Filesystem: Debug {
    fn get_id(&self) -> String;
    fn copy(&self) -> Box<Filesystem>;
    fn eq(&self, other: &Filesystem) -> bool;
}

impl PartialEq for Filesystem {
    fn eq(&self, other: &Filesystem) -> bool {
        self.eq(other)
    }
}

pub trait Pool: Debug {
    fn create_filesystem(&mut self,
                         filesystem_name: &str,
                         mount_point: &str,
                         size: u64)
                         -> EngineResult<()>;
    fn add_blockdev(&mut self, path: &Path) -> EngineResult<()>;
    fn remove_blockdev(&mut self, path: &Path) -> EngineResult<()>;
    fn add_cachedev(&mut self, path: &Path) -> EngineResult<()>;
    fn remove_cachedev(&mut self, path: &Path) -> EngineResult<()>;
    fn destroy(&mut self) -> EngineResult<()>;
    fn list_filesystems(&self) -> EngineResult<BidirMap<String, Box<Filesystem>>>;
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
}

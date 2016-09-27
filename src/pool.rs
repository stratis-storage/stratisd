// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.


use std::sync::Arc;
use std::cell::Cell;
use std::path::PathBuf;

use dbus::{tree, Path};

use types::StratisResult;
use blockdev::BlockDevs;

pub trait StratisPool {
    fn add_blockdev(&mut self, path: &str) -> StratisResult<()>;
    fn add_cachedev(&mut self, path: &str) -> StratisResult<()>;
    fn destroy(&mut self) -> StratisResult<()>;
}

#[derive(Debug)]
pub struct Pool {
    pub name: String,
    pub path: Path<'static>,
    pub block_devs: BlockDevs,
    pub index: i32,
    pub online: Cell<bool>,
    pub checking: Cell<bool>,
}


impl Pool {
    // pub fn new_pool(name: &str, blockdev_paths: &[PathBuf], raid_level: u16) -> Pool {
    //
    // let block_devs = BlockDevs::new(blockdev_paths);
    //
    // Pool {
    // name: name.to_owned(),
    // block_devs: block_devs.to_owned(),
    // TODO use a constant for object path
    // path: format!("/org/storage/stratis/{}", index).into(),
    // index: index,
    // online: Cell::new(index % 2 == 0),
    // checking: Cell::new(false),
    // }
    // }
}

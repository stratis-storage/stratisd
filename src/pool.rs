// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.


use std::path::PathBuf;

use types::StratisResult;
use blockdev::BlockDevs;

pub trait StratisPool {
    fn add_blockdev(&mut self, path: &str) -> StratisResult<()>;
    fn add_cachedev(&mut self, path: &str) -> StratisResult<()>;
    fn destroy(&mut self) -> StratisResult<()>;
    fn get_name(&mut self) -> String;
    fn copy(&self) -> Box<StratisPool>;
}

#[derive(Clone)]
pub struct Pool {
    pub name: String,
    pub block_devs: BlockDevs,
    pub raid_level: u16,
    pub online: bool,
    pub checking: bool,
}

impl Pool {
    pub fn new_pool(name: &str, blockdev_paths: &[PathBuf], raid_level: u16) -> Box<StratisPool> {

        let status = BlockDevs::new(blockdev_paths);

        let block_devs = status.unwrap();

        let new_pool = Pool {
            name: name.to_owned(),
            block_devs: block_devs.to_owned(),
            raid_level: raid_level,
            online: true,
            checking: false,
        };

        Box::new(new_pool)
    }
}

impl StratisPool for Pool {
    fn copy(&self) -> Box<StratisPool> {
        let pool_copy = Pool {
            name: self.name.clone(),
            block_devs: self.block_devs.clone(),
            raid_level: self.raid_level.clone(),
            online: true,
            checking: false,
        };
        Box::new(pool_copy)
    }

    fn add_blockdev(&mut self, path: &str) -> StratisResult<()> {
        println!("sim: pool::add_blockdev");
        Ok(())
    }

    fn add_cachedev(&mut self, path: &str) -> StratisResult<()> {
        println!("sim: pool::add_cachedev");
        Ok(())
    }

    fn destroy(&mut self) -> StratisResult<()> {
        println!("sim: pool::destroy");
        Ok(())
    }

    fn get_name(&mut self) -> String {
        self.name.clone()
    }
}

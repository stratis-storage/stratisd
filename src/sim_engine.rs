// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use blockdev::BlockDevs;

use engine::Engine;

use pool::StratisPool;

use std::path::Path;
use std::collections::BTreeMap;

use types::StratisResult;



pub struct SimEngine {
    pub pools: BTreeMap<String, Box<StratisPool>>,
}

impl SimEngine {
    pub fn new() -> SimEngine {
        SimEngine { pools: BTreeMap::new() }
    }
}

impl Engine for SimEngine {
    fn create_pool(&mut self,
                   name: &str,
                   blockdev_paths: &[&Path],
                   raid_level: u16)
                   -> StratisResult<()> {

        let pool = SimPool::new_pool(name, blockdev_paths, raid_level);

        self.pools.insert(name.to_owned(), pool);

        Ok(())
    }

    fn destroy_pool(&mut self, name: &str) -> StratisResult<()> {

        self.pools.remove(name);

        Ok(())
    }

    fn list_pools(&self) -> StratisResult<BTreeMap<String, Box<StratisPool>>> {

        let mut stratis_pools = BTreeMap::new();

        for (name, pool) in &self.pools {

            stratis_pools.insert(name.clone(), pool.copy());
        }

        Ok(stratis_pools)
    }
}

pub struct SimPool {
    pub name: String,
    pub block_devs: BlockDevs,
    pub raid_level: u16,
    pub online: bool,
    pub checking: bool,
}

impl SimPool {
    pub fn new_pool(name: &str, blockdev_paths: &[&Path], raid_level: u16) -> Box<StratisPool> {

        let status = BlockDevs::new(blockdev_paths);

        let block_devs = status.unwrap();

        let new_pool = SimPool {
            name: name.to_owned(),
            block_devs: block_devs.to_owned(),
            raid_level: raid_level,
            online: true,
            checking: false,
        };

        Box::new(new_pool)
    }
}

impl StratisPool for SimPool {
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
    fn copy(&self) -> Box<StratisPool> {
        let pool_copy = SimPool {
            name: self.name.clone(),
            block_devs: self.block_devs.clone(),
            raid_level: self.raid_level.clone(),
            online: true,
            checking: false,
        };
        Box::new(pool_copy)
    }
}

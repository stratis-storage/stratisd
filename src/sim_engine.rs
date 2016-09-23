// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use types::StratisResult;
use engine::{Engine, Pool};


pub struct SimEngine {

}

impl SimEngine {
    pub fn new() -> SimEngine {
        SimEngine {}
    }
}

impl Engine for SimEngine {
    fn create_pool(&self,
                   name: &str,
                   blockdev_paths: &[&str],
                   raid_level: i32)
                   -> StratisResult<Box<Pool>> {
        println!("sim: pool created");

        Ok(Box::new(SimPool::new()))
    }
}

struct SimPool {
    tmp: u32,
}

impl SimPool {
    fn new() -> SimPool {
        SimPool { tmp: 4 }
    }
}

impl Pool for SimPool {
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
}

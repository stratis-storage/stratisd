// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;
use std::collections::BTreeMap;
use std::iter::FromIterator;

use engine::Engine;
use engine::EngineError;
use engine::EngineResult;
use engine::ErrorEnum;
use engine::Pool;

use super::consts::*;
use super::blockdev::BlockDev;
use super::pool::StratPool;


#[derive(Debug)]
pub struct StratEngine {
    pub pools: BTreeMap<String, StratPool>,
}

impl StratEngine {
    pub fn new() -> StratEngine {
        StratEngine { pools: BTreeMap::new() }
    }
}

impl Engine for StratEngine {
    fn configure_simulator(&mut self, _denominator: u32) -> EngineResult<()> {
        Ok(()) // we're not the simulator and not configurable, so just say ok
    }

    fn create_pool(&mut self,
                   name: &str,
                   blockdev_paths: &[&Path],
                   raid_level: u16)
                   -> EngineResult<()> {

        if self.pools.contains_key(name) {
            return Err(EngineError::Stratis(ErrorEnum::AlreadyExists(name.into())));
        }

        let mut devs = Vec::new();
        for path in blockdev_paths {
            match BlockDev::new(name, path, MIN_MDA_SIZE, true) {
                Ok(bd) => devs.push(bd),
                Err(e) => {
                    for mut dev in devs {
                        let _dontcare = dev.wipe_sigblock();
                    }
                    return Err(e);
                }
            }
        }

        let pool = StratPool::new(name, &devs, raid_level);

        self.pools.insert(name.to_owned(), pool);
        Ok(())
    }

    fn destroy_pool(&mut self, name: &str) -> EngineResult<()> {
        self.pools.remove(name);

        Ok(())
    }

    fn get_pool(&mut self, name: &str) -> EngineResult<&mut Pool> {
        match self.pools.get_mut(name) {
            Some(pool) => Ok(pool),
            None => Err(EngineError::Stratis(ErrorEnum::NotFound(name.into()))),
        }
    }

    fn pools(&mut self) -> BTreeMap<&str, &mut Pool> {
        BTreeMap::from_iter(self.pools.iter_mut().map(|x| (x.0 as &str, x.1 as &mut Pool)))
    }
}

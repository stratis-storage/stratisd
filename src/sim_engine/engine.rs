// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use engine::Engine;
use engine::EngineError;
use engine::EngineResult;
use engine::ErrorEnum;

use engine::Pool;

use rand::Rng;
use rand::ThreadRng;
use rand::thread_rng;

use std::fmt;

use std::path::Path;
use std::collections::BTreeMap;
use std::iter::FromIterator;

use super::pool::SimPool;



pub struct SimEngine {
    pub pools: BTreeMap<String, Box<Pool>>,
    rng: ThreadRng,
}

impl fmt::Debug for SimEngine {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{{SimEngine {:?}", self.pools)
    }
}

impl SimEngine {
    pub fn new() -> SimEngine {
        SimEngine {
            pools: BTreeMap::new(),
            rng: thread_rng(),
        }
    }
}

impl Engine for SimEngine {
    fn create_pool(&mut self,
                   name: &str,
                   blockdev_paths: &[&Path],
                   raid_level: u16)
                   -> EngineResult<()> {

        if self.pools.contains_key(name) {
            return Err(EngineError::Stratis(ErrorEnum::AlreadyExists(name.into())));
        }

        let bad_blockdev_path = if self.rng.gen_weighted_bool(8) {
            self.rng.choose(blockdev_paths)
        } else {
            None
        };
        match bad_blockdev_path {
            Some(x) => {
                let path_as_str = x.to_str().unwrap_or("unstringable path");
                return Err(EngineError::Stratis(ErrorEnum::Busy(path_as_str.into())));
            }
            None => {}
        }

        let pool = SimPool::new_pool(name, blockdev_paths, raid_level);

        if self.rng.gen_weighted_bool(8) {
            return Err(EngineError::Stratis(ErrorEnum::Error("X".into())));
        }

        self.pools.insert(name.to_owned(), pool);
        Ok(())
    }

    fn destroy_pool(&mut self, name: &str) -> EngineResult<()> {
        if self.rng.gen_weighted_bool(8) {
            return Err(EngineError::Stratis(ErrorEnum::Busy("X".into())));
        }

        self.pools.remove(name);

        Ok(())
    }

    fn list_pools(&self) -> EngineResult<BTreeMap<String, Box<Pool>>> {

        Ok(BTreeMap::from_iter(self.pools.iter().map(|x| (x.0.clone(), x.1.copy()))))

    }
}

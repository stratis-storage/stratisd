// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::iter::FromIterator;
use std::path::Path;
use std::path::PathBuf;

use uuid::Uuid;

use engine::Engine;
use engine::EngineError;
use engine::EngineResult;
use engine::ErrorEnum;
use engine::RenameAction;
use engine::Pool;

use super::pool::StratPool;

#[derive(Debug)]
pub enum DevOwnership {
    Ours(Uuid),
    Unowned,
    Theirs,
}

#[derive(Debug)]
pub struct StratEngine {
    pools: BTreeMap<String, StratPool>,
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
                   raid_level: u16,
                   force: bool)
                   -> EngineResult<Vec<PathBuf>> {

        if self.pools.contains_key(name) {
            return Err(EngineError::Stratis(ErrorEnum::AlreadyExists(name.into())));
        }
        let pool = try!(StratPool::new(name, blockdev_paths, raid_level, force));
        let bdev_paths = pool.block_devs.iter().map(|p| p.1.devnode.clone()).collect();

        self.pools.insert(name.to_owned(), pool);
        Ok(bdev_paths)
    }

    /// Destroy a pool, if the pool does not exist, return Ok.
    fn destroy_pool(&mut self, name: &str) -> EngineResult<bool> {
        destroy_pool!{self; name}
    }

    fn rename_pool(&mut self, old_name: &str, new_name: &str) -> EngineResult<RenameAction> {
        rename_pool!{self; old_name; new_name}
    }

    fn get_pool(&mut self, name: &str) -> EngineResult<&mut Pool> {
        get_pool!(self; name)
    }

    fn pools(&mut self) -> BTreeMap<&str, &mut Pool> {
        pools!(self)
    }
}

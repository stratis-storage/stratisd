// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;
use std::path::PathBuf;

use devicemapper::DM;
use uuid::Uuid;

use engine::Engine;
use engine::EngineError;
use engine::EngineResult;
use engine::ErrorEnum;
use engine::Pool;
use engine::Redundancy;
use engine::RenameAction;

use super::pool::StratPool;

use super::super::engine::{HasName, HasUuid, PoolUuid};
use super::super::structures::Table;

#[derive(Debug, PartialEq, Eq)]
pub enum DevOwnership {
    Ours(Uuid),
    Unowned,
    Theirs,
}

#[derive(Debug)]
pub struct StratEngine {
    pools: Table<StratPool>,
}

impl StratEngine {
    pub fn new() -> StratEngine {
        StratEngine { pools: Table::new() }
    }

    /// Teardown Stratis, preparatory to a shutdown.
    pub fn teardown(self) -> EngineResult<()> {
        for pool in self.pools.empty() {
            try!(pool.teardown());
        }
        Ok(())
    }
}

impl Engine for StratEngine {
    fn configure_simulator(&mut self, _denominator: u32) -> EngineResult<()> {
        Ok(()) // we're not the simulator and not configurable, so just say ok
    }

    fn create_pool(&mut self,
                   name: &str,
                   blockdev_paths: &[&Path],
                   redundancy: Option<u16>,
                   force: bool)
                   -> EngineResult<(PoolUuid, Vec<PathBuf>)> {

        let redundancy = calculate_redundancy!(redundancy);

        if self.pools.contains_name(name) {
            return Err(EngineError::Engine(ErrorEnum::AlreadyExists, name.into()));
        }

        let dm = try!(DM::new());
        let pool = try!(StratPool::initialize(name, &dm, blockdev_paths, redundancy, force));
        let bdev_devnodes = pool.block_devs.devnodes();

        let uuid = pool.uuid().clone();
        self.pools.insert(pool);
        Ok((uuid, bdev_devnodes))
    }

    fn destroy_pool(&mut self, uuid: &PoolUuid) -> EngineResult<bool> {
        destroy_pool!{self; uuid}
    }

    fn rename_pool(&mut self, uuid: &PoolUuid, new_name: &str) -> EngineResult<RenameAction> {
        rename_pool!{self; uuid; new_name}
    }

    fn get_pool(&mut self, uuid: &PoolUuid) -> Option<&mut Pool> {
        get_pool!(self; uuid)
    }

    fn check(&mut self) -> () {
        for pool in self.pools.iter_mut() {
            pool.check();
        }
    }
}

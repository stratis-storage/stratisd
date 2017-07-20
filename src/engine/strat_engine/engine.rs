// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fs::create_dir;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use uuid::Uuid;

use devicemapper::DM;

use super::super::engine::{Engine, HasName, HasUuid, Pool};
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::structures::Table;
use super::super::types::{PoolUuid, Redundancy, RenameAction};

use super::cleanup::teardown_pools;
use super::pool::StratPool;
use super::setup::find_all;

pub const DEV_PATH: &'static str = "/dev/stratis";

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
    /// Setup a StratEngine.
    /// 1. Verify the existance of Stratis /dev directory.
    /// 2. Setup all the pools belonging to the engine.
    ///
    /// Returns an error if there was an error reading device nodes.
    /// Returns an error if there was an error setting up any of the pools.
    pub fn initialize() -> EngineResult<StratEngine> {
        if let Err(err) = create_dir(DEV_PATH) {
            if err.kind() != ErrorKind::AlreadyExists {
                return Err(From::from(err));
            }
        }

        let pools = try!(find_all());

        let mut table = Table::default();
        for (pool_uuid, devices) in &pools {
            let evicted = table.insert(try!(StratPool::setup(*pool_uuid, devices)));
            if !evicted.is_empty() {

                // TODO: update state machine on failure.
                let _ = teardown_pools(table.empty());

                let err_msg = "found two pools with the same id or name";
                return Err(EngineError::Engine(ErrorEnum::Invalid, err_msg.into()));
            }
        }

        Ok(StratEngine { pools: table })
    }

    /// Teardown Stratis, preparatory to a shutdown.
    pub fn teardown(self) -> EngineResult<()> {
        Ok(try!(teardown_pools(self.pools.empty())))
    }

    /// Get pool as StratPool
    pub fn get_strat_pool(&self, uuid: &PoolUuid) -> Option<&StratPool> {
        self.pools.get_by_uuid(uuid)
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
        let (pool, devnodes) =
            try!(StratPool::initialize(name, &dm, blockdev_paths, redundancy, force));

        let uuid = pool.uuid().clone();
        self.pools.insert(pool);
        Ok((uuid, devnodes))
    }

    fn destroy_pool(&mut self, uuid: &PoolUuid) -> EngineResult<bool> {
        destroy_pool!{self; uuid}
    }

    fn rename_pool(&mut self, uuid: &PoolUuid, new_name: &str) -> EngineResult<RenameAction> {
        let old_name = rename_pool_pre!(self; uuid; new_name);

        let mut pool = self.pools
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.pools.get_by_uuid() returned a value.");
        pool.rename(new_name);

        if let Err(err) = pool.write_metadata() {
            pool.rename(&old_name);
            self.pools.insert(pool);
            Err(err)
        } else {
            self.pools.insert(pool);
            Ok(RenameAction::Renamed)
        }
    }

    fn get_pool(&mut self, uuid: &PoolUuid) -> Option<&mut Pool> {
        get_pool!(self; uuid)
    }

    fn check(&mut self) -> () {
        check_engine!(self);
    }
}

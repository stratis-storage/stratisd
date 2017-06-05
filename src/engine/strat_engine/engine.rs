// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fs::create_dir;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use devicemapper::DM;
use uuid::Uuid;


use super::super::engine::{Engine, HasName, HasUuid, Pool};
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::structures::Table;
use super::super::types::{PoolUuid, Redundancy, RenameAction};

use super::pool::StratPool;
use super::setup::{find_all, get_pool_blockdevs, get_pool_dmdevs, get_pool_metadata,
                   get_pool_filesystems};

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
    pub fn initialize() -> EngineResult<StratEngine> {
        if let Err(err) = create_dir(DEV_PATH) {
            if err.kind() != ErrorKind::AlreadyExists {
                return Err(From::from(err));
            }
        }

        Ok(StratEngine { pools: Table::new() })
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
        check_engine!(self);
    }

    // TODO: Fix this method so that it actually sets up the engine.
    fn setup(&mut self) -> EngineResult<()> {
        let pools = try!(find_all());
        if pools.is_empty() {
            return Ok(());
        }

        let metadata = try!(get_pool_metadata(&pools));
        let blockdevs = try!(get_pool_blockdevs(&pools, &metadata));
        let stuff = try!(get_pool_dmdevs(&blockdevs, &metadata));
        let filesystems = try!(get_pool_filesystems(&stuff));
        // TODO: all the pool pieces acquired, now build some
        // StratPools, make a StratEngine and return it as an Engine

        let err_msg = "Stratis was already run once, can not yet reconstruct state";
        Err(EngineError::Engine(ErrorEnum::AlreadyExists, err_msg.into()))
    }
}

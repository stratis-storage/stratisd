// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle the physical backing store of a pool.

use std::collections::HashMap;
use std::path::Path;

use devicemapper::{Device, Sectors};

use super::super::super::engine::BlockDev;
use super::super::super::errors::EngineResult;
use super::super::super::types::{DevUuid, PoolUuid};

use super::super::serde_structs::{Recordable, BlockDevSave};

use super::blockdevmgr::{BlkDevSegment, BlockDevMgr};

#[derive(Debug)]
pub struct Store {
    block_mgr: BlockDevMgr,
}

impl Store {
    /// Make a Store object from blockdevs that already belong to Stratis.
    pub fn new(block_mgr: BlockDevMgr) -> Store {
        Store { block_mgr: block_mgr }
    }

    /// Initialize a Store object, by initializing the specified devs.
    pub fn initialize(pool_uuid: PoolUuid,
                      paths: &[&Path],
                      mda_size: Sectors,
                      force: bool)
                      -> EngineResult<Store> {
        Ok(Store::new(BlockDevMgr::initialize(pool_uuid, paths, mda_size, force)?))
    }

    pub fn add(&mut self, paths: &[&Path], force: bool) -> EngineResult<Vec<DevUuid>> {
        self.block_mgr.add(paths, force)
    }

    pub fn alloc_space(&mut self, sizes: &[Sectors]) -> Option<Vec<Vec<BlkDevSegment>>> {
        self.block_mgr.alloc_space(sizes)
    }

    pub fn blockdevs(&self) -> Vec<(DevUuid, &BlockDev)> {
        self.block_mgr.blockdevs()
    }

    pub fn current_capacity(&self) -> Sectors {
        self.block_mgr.current_capacity()
    }

    pub fn destroy_all(self) -> EngineResult<()> {
        self.block_mgr.destroy_all()
    }

    pub fn get_blockdev_by_uuid(&self, uuid: DevUuid) -> Option<&BlockDev> {
        self.block_mgr.get_blockdev_by_uuid(uuid)
    }

    pub fn get_mut_blockdev_by_uuid(&mut self, uuid: DevUuid) -> Option<&mut BlockDev> {
        self.block_mgr.get_mut_blockdev_by_uuid(uuid)
    }

    pub fn metadata_size(&self) -> Sectors {
        self.block_mgr.metadata_size()
    }

    pub fn save_state(&mut self, metadata: &[u8]) -> EngineResult<()> {
        self.block_mgr.save_state(metadata)
    }

    pub fn uuid_to_devno(&self) -> Box<Fn(DevUuid) -> Option<Device>> {
        self.block_mgr.uuid_to_devno()
    }
}

impl Recordable<HashMap<DevUuid, BlockDevSave>> for Store {
    fn record(&self) -> HashMap<DevUuid, BlockDevSave> {
        self.block_mgr.record()
    }
}

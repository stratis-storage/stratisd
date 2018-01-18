// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle the physical backing store of a pool.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};

use devicemapper::{CacheDev, Device, Sectors};

use super::super::super::engine::BlockDev;
use super::super::super::errors::EngineResult;
use super::super::super::types::{DevUuid, PoolUuid};

use super::super::serde_structs::{Recordable, BlockDevSave};

use super::blockdev::StratBlockDev;
use super::blockdevmgr::{BlkDevSegment, BlockDevMgr};

#[derive(Debug)]
pub struct Store {
    block_mgr: BlockDevMgr,
    cache: Option<CacheDev>,
}

impl Store {
    /// Construct an object that represents an existing physical layer.
    pub fn new(pool_uuid: PoolUuid,
               block_devs: Vec<StratBlockDev>,
               cache: Option<CacheDev>,
               last_update_time: Option<DateTime<Utc>>)
               -> Store {
        Store {
            block_mgr: BlockDevMgr::new(pool_uuid, block_devs, last_update_time),
            cache: cache,
        }
    }

    /// Initialize the physical layer.
    pub fn initialize(pool_uuid: PoolUuid,
                      paths: &[&Path],
                      mda_size: Sectors,
                      cache: Option<CacheDev>,
                      force: bool)
                      -> EngineResult<Store> {
        Ok(Store {
               block_mgr: BlockDevMgr::initialize(pool_uuid, paths, mda_size, force)?,
               cache: cache,
           })
    }

    /// Add additional blockdevs to the physical layer.
    /// This may or may not cause the cache layer to be extended.
    pub fn add(&mut self, paths: &[&Path], force: bool) -> EngineResult<Vec<DevUuid>> {
        self.block_mgr.add(paths, force)
    }

    /// Allocate space from the physical layer.
    /// This space can reside either on the optional cache, or on the blockdevs
    /// which comprise the base of the physical layer.
    pub fn alloc_space(&mut self, sizes: &[Sectors]) -> Option<Vec<Vec<BlkDevSegment>>> {
        if let Some(ref _cache) = self.cache {
            unimplemented!()
        } else {
            self.block_mgr.alloc_space(sizes)
        }
    }

    /// Return a reference to the blockdevs that form the base of the physical
    /// layer.
    // The basic use is so that information about the blockdevs can
    // be shown in the D-Bus API.
    pub fn blockdevs(&self) -> Vec<&BlockDev> {
        self.block_mgr.blockdevs()
    }

    /// The current capacity of all the blockdevs that make up the physical
    /// layer. This should be greater than the size of the cache device, which
    /// does not include the segments of the blockdevs that contain Stratis
    /// metadata.
    pub fn current_capacity(&self) -> Sectors {
        self.block_mgr.current_capacity()
    }

    /// Destroy the physical layer completely. When done, the components of
    /// the layer should be entirely eliminated, in the case of the cache,
    /// or completely free to be reused.
    pub fn destroy_all(self) -> EngineResult<()> {
        if let Some(ref _cache) = self.cache {
            unimplemented!()
        }
        self.block_mgr.destroy_all()
    }

    /// Lookup an immutable blockdev by its Stratis UUID.
    // Used for obtaining properties of a blockdev via the D-Bus.
    pub fn get_blockdev_by_uuid(&self, uuid: DevUuid) -> Option<&BlockDev> {
        self.block_mgr.get_blockdev_by_uuid(uuid)
    }

    /// Lookup a mutable blockdev by its Stratis UUID.
    // Used for setting properties of a blockdev via the D-Bus.
    pub fn get_mut_blockdev_by_uuid(&mut self, uuid: DevUuid) -> Option<&mut BlockDev> {
        self.block_mgr.get_mut_blockdev_by_uuid(uuid)
    }

    /// The number of sectors in the physical layer given up to Stratis
    /// metadata. current_capacity() - metadata_size() == the size of the
    /// cache device.
    pub fn metadata_size(&self) -> Sectors {
        self.block_mgr.metadata_size()
    }

    /// Write the given data directly to the blockdevs that make up the
    /// physical layer. This action bypasses the cache entirely, in order
    /// to allow control over which blockdevs the metadata is written to.
    pub fn save_state(&mut self, metadata: &[u8]) -> EngineResult<()> {
        self.block_mgr.save_state(metadata)
    }

    /// Map a Stratis blockdev uuid to a corresponding devnode.
    // Used during setup to construct devicemapper tables from Stratis
    // metadata.
    pub fn uuid_to_devno(&self) -> Box<Fn(DevUuid) -> Option<Device>> {
        self.block_mgr.uuid_to_devno()
    }
}

impl Recordable<HashMap<DevUuid, BlockDevSave>> for Store {
    fn record(&self) -> HashMap<DevUuid, BlockDevSave> {
        self.block_mgr.record()
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle the physical backing store of a pool.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use devicemapper::{Device, DM, DmDevice, LinearDev, Sectors};

use super::super::super::engine::BlockDev;
use super::super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::super::types::{DevUuid, PoolUuid};

use super::super::dmnames::{PhysicalRole, format_physical_name};
use super::super::serde_structs::{Recordable, StoreSave};

use super::blockdevmgr::{BlkDevSegment, BlockDevMgr, Segment, get_coalesced_segments, map_to_dm};
use super::setup::get_blockdevs;

/// Handles the lowest level, base layer of this tier.
/// The dm_device organizes all block devs into a single linear allocation pool.
#[derive(Debug)]
struct DataLayer {
    /// Manages the individual block devices
    block_mgr: BlockDevMgr,
    /// The device mapper device which aggregates all block_mgr's devices
    dm_device: LinearDev,
    /// The list of segments granted by block_mgr and used by dm_device
    /// It is always the case that block_mgr.avail_space() == 0, i.e., all
    /// available space in block_mgr is allocated to the dm_device.
    segments: Vec<BlkDevSegment>,
    /// The position from which requested space is allocated
    next: Sectors,
}

impl DataLayer {
    /// Setup a previously existing data layer from the block_mgr and
    /// previously allocated segments. There is a possibility that the
    /// size of the device has changed for the bigger since the last time
    /// its metadata was recorded, so allocate any unallocated segments that
    /// might have resulted from this.
    pub fn setup(dm: &DM,
                 mut block_mgr: BlockDevMgr,
                 segments: &[(DevUuid, Sectors, Sectors)],
                 next: Sectors)
                 -> EngineResult<DataLayer> {
        let uuid_to_devno = block_mgr.uuid_to_devno();
        let mapper = |triple: &(DevUuid, Sectors, Sectors)| -> EngineResult<BlkDevSegment> {
            let device = uuid_to_devno(triple.0)
                .ok_or_else(|| {
                                EngineError::Engine(ErrorEnum::NotFound,
                                                    format!("missing device for UUUD {:?}",
                                                            &triple.0))
                            })?;
            Ok(BlkDevSegment::new(triple.0, Segment::new(device, triple.1, triple.2)))
        };
        let mut segments = segments
            .iter()
            .map(&mapper)
            .collect::<EngineResult<Vec<_>>>()?;

        let mut ld = LinearDev::setup(dm,
                                      &format_physical_name(block_mgr.pool_uuid(),
                                                            PhysicalRole::Origin),
                                      None,
                                      map_to_dm(&segments))?;

        let avail_space = block_mgr.avail_space();
        let additional_segments = block_mgr
            .alloc_space(&[avail_space])
            .expect("asked for exactly the space available, must get")
            .iter()
            .flat_map(|s| s.iter())
            .cloned()
            .collect::<Vec<_>>();

        // Add the new segments only after setting up the linear dev.
        // Otherwise the new table will differ from the old and setup of the
        // dev will fail.
        if !additional_segments.is_empty() {
            segments.extend(additional_segments);
            ld.set_table(dm, map_to_dm(&segments))?;
        }

        Ok(DataLayer {
               block_mgr,
               dm_device: ld,
               segments,
               next,
           })
    }


    /// Setup a new DataLayer struct from the block_mgr.
    /// Note that this is a metadata changing event.
    pub fn new(dm: &DM, mut block_mgr: BlockDevMgr) -> EngineResult<DataLayer> {
        let avail_space = block_mgr.avail_space();
        let segments = block_mgr
            .alloc_space(&[avail_space])
            .expect("asked for exactly the space available, must get")
            .iter()
            .flat_map(|s| s.iter())
            .cloned()
            .collect::<Vec<_>>();
        let ld = LinearDev::setup(dm,
                                  &format_physical_name(block_mgr.pool_uuid(),
                                                        PhysicalRole::Origin),
                                  None,
                                  map_to_dm(&segments))?;
        Ok(DataLayer {
               block_mgr,
               dm_device: ld,
               segments,
               next: Sectors(0),
           })
    }

    /// Add the given paths to self. Return UUIDs of the new blockdevs
    /// corresponding to the specified paths.
    /// Note that this is a metadata changing event.
    pub fn add(&mut self, dm: &DM, paths: &[&Path], force: bool) -> EngineResult<Vec<DevUuid>> {
        let uuids = self.block_mgr.add(paths, force)?;

        let avail_space = self.block_mgr.avail_space();
        let segments = self.block_mgr
            .alloc_space(&[avail_space])
            .expect("asked for exactly the space available, must get")
            .iter()
            .flat_map(|s| s.iter())
            .cloned()
            .collect::<Vec<_>>();
        let coalesced = get_coalesced_segments(&self.segments, &segments);

        self.dm_device.set_table(dm, map_to_dm(&coalesced))?;

        self.segments = coalesced;

        Ok(uuids)
    }

    /// All the sectors available to this device
    // Note that this should always be equivalent to the sum of the length
    // fields of the segments in self.segments.
    #[allow(dead_code)]
    pub fn capacity(&self) -> Sectors {
        self.dm_device.size()
    }

    /// Number of sectors unused
    #[allow(dead_code)]
    pub fn available(&self) -> Sectors {
        self.capacity() - self.next
    }

    /// Allocate requested chunks from device.
    /// Returns None if it is not possible to satisfy the request.
    // Simply serves up the space in the order in which it was requested.
    #[allow(dead_code)]
    pub fn alloc_space(&mut self, sizes: &[Sectors]) -> Option<Vec<Vec<(Sectors, Sectors)>>> {
        if self.available() < sizes.iter().cloned().sum() {
            return None;
        }

        let mut chunks = Vec::new();
        for size in sizes {
            chunks.push(vec![(self.next, *size)]);
            self.next += *size;
        }
        Some(chunks)
    }

    /// Destroy the store. Teardown its DM devices and wipe its blockdevs.
    pub fn destroy(self, dm: &DM) -> EngineResult<()> {
        self.dm_device.teardown(dm)?;
        self.block_mgr.destroy_all()
    }
}

#[derive(Debug)]
pub struct Store {
    data: DataLayer,
}

impl Store {
    /// Make a Store object from blockdevs that already belong to Stratis.
    pub fn setup(dm: &DM,
                 pool_uuid: PoolUuid,
                 store_save: &StoreSave,
                 devnodes: &HashMap<Device, PathBuf>,
                 last_update_time: Option<DateTime<Utc>>)
                 -> EngineResult<Store> {
        let blockdevs = get_blockdevs(pool_uuid, store_save, devnodes)?;
        let block_mgr = BlockDevMgr::new(pool_uuid, blockdevs, last_update_time);
        Ok(Store { data: DataLayer::setup(dm, block_mgr, &store_save.segments, store_save.next)? })
    }

    /// Initialize a Store object, by initializing the specified devs.
    pub fn initialize(dm: &DM,
                      pool_uuid: PoolUuid,
                      paths: &[&Path],
                      mda_size: Sectors,
                      force: bool)
                      -> EngineResult<Store> {
        Ok(Store {
               data: DataLayer::new(dm,
                                    BlockDevMgr::initialize(pool_uuid, paths, mda_size, force)?)?,
           })
    }

    pub fn add(&mut self, dm: &DM, paths: &[&Path], force: bool) -> EngineResult<Vec<DevUuid>> {
        self.data.add(dm, paths, force)
    }

    /// Allocate space from the underlying device.
    pub fn alloc_space(&mut self, sizes: &[Sectors]) -> Option<Vec<Vec<(Sectors, Sectors)>>> {
        self.data.alloc_space(sizes)
    }

    /// Return a reference to the blockdevs that form the base of the physical
    /// layer.
    pub fn blockdevs(&self) -> Vec<(DevUuid, &BlockDev)> {
        self.data.block_mgr.blockdevs()
    }

    /// The current capacity of all the blockdevs that make up the physical
    /// layer. This should be greater than the size of the dm device, which
    /// does not include the sectors of the blockdevs that contain Stratis
    /// metadata.
    pub fn current_capacity(&self) -> Sectors {
        self.data.block_mgr.current_capacity()
    }

    /// Destroy the entire store.
    pub fn destroy(self, dm: &DM) -> EngineResult<()> {
        self.data.destroy(dm)
    }

    /// Return the device that this tier is currently using.
    /// Note that this may change if it switches between its cache and non-
    /// cache incarnataions.
    pub fn device(&self) -> Device {
        self.data.dm_device.device()
    }


    /// Lookup an immutable blockdev by its Stratis UUID.
    // Used for getting properties of a blockdev via the D-Bus.
    pub fn get_blockdev_by_uuid(&self, uuid: DevUuid) -> Option<&BlockDev> {
        self.data.block_mgr.get_blockdev_by_uuid(uuid)
    }

    /// Lookup a mutable blockdev by its Stratis UUID.
    // Used for setting properties of a blockdev via the D-Bus.
    pub fn get_mut_blockdev_by_uuid(&mut self, uuid: DevUuid) -> Option<&mut BlockDev> {
        self.data.block_mgr.get_mut_blockdev_by_uuid(uuid)
    }

    /// the number of sectors in the physical layer given up to Stratis
    /// metadata. current_capacity() - metadata_size() >= the size of the
    /// DM device.
    pub fn metadata_size(&self) -> Sectors {
        self.data.block_mgr.metadata_size()
    }

    /// Write the given data directly to the blockdevs that make up the
    /// physical layer. This action bypasses the DM device entirely,
    /// in order to allow control over which blockdevs the metadata is written
    /// to.
    pub fn save_state(&mut self, metadata: &[u8]) -> EngineResult<()> {
        self.data.block_mgr.save_state(metadata)
    }
}

impl Recordable<StoreSave> for Store {
    fn record(&self) -> StoreSave {
        StoreSave {
            segments: self.data.segments.record(),
            block_devs: self.data.block_mgr.record(),
            next: self.data.next,
        }
    }
}

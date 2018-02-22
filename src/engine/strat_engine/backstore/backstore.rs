// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle the backing store of a pool.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use devicemapper::{DM, Device, DmDevice, LinearDev, Sectors};

use super::super::super::engine::BlockDev;
use super::super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::super::types::{DevUuid, PoolUuid};

use super::super::dmnames::{CacheRole, format_backstore_ids};
use super::super::serde_structs::{BackstoreSave, Recordable};

use super::blockdevmgr::{BlkDevSegment, BlockDevMgr, Segment, coalesce_blkdevsegs, map_to_dm};
use super::setup::get_blockdevs;

/// Handles the lowest level, base layer of this tier.
/// The dm_device organizes all block devs into a single linear allocation
/// pool. This structure can allocate additional space to the upper layer,
/// but it cannot accept returned space. When it is extended to be able to
/// accept returned space the allocation algorithm will have to be revised.
/// All available sectors on blockdevs in the manager are allocated to
/// dm_device.
#[derive(Debug)]
struct DataTier {
    /// Manages the individual block devices
    /// it is always the case block_mgr.avail_space() == 0.
    block_mgr: BlockDevMgr,
    /// The device mapper device which aggregates block_mgr's devices
    dm_device: LinearDev,
    /// The list of segments granted by block_mgr and used by dm_device
    segments: Vec<BlkDevSegment>,
    /// The position from which requested space is allocated
    next: Sectors,
}

impl DataTier {
    /// Setup a previously existing data layer from the block_mgr and
    /// previously allocated segments. There is a possibility that the
    /// size of some blockdev has changed for the bigger since the last time
    /// its metadata was recorded, so allocate any unallocated segments that
    /// might have resulted from this.
    /// WARNING: metadata changing event
    pub fn setup(dm: &DM,
                 block_mgr: BlockDevMgr,
                 segments: &[(DevUuid, Sectors, Sectors)],
                 next: Sectors)
                 -> EngineResult<DataTier> {
        if block_mgr.avail_space() != Sectors(0) {
            let err_msg = format!("{} unallocated to device; probable metadata corruption",
                                  block_mgr.avail_space());
            return Err(EngineError::Engine(ErrorEnum::Error, err_msg));
        }

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
        let segments = segments
            .iter()
            .map(&mapper)
            .collect::<EngineResult<Vec<_>>>()?;

        let (dm_name, dm_uuid) = format_backstore_ids(block_mgr.pool_uuid(), CacheRole::OriginSub);
        let ld = LinearDev::setup(dm, &dm_name, Some(&dm_uuid), map_to_dm(&segments))?;

        Ok(DataTier {
               block_mgr,
               dm_device: ld,
               segments,
               next,
           })
    }


    /// Setup a new DataTier struct from the block_mgr.
    /// WARNING: metadata changing event
    pub fn new(dm: &DM, mut block_mgr: BlockDevMgr) -> EngineResult<DataTier> {
        let avail_space = block_mgr.avail_space();
        let segments = block_mgr
            .alloc_space(&[avail_space])
            .expect("asked for exactly the space available, must get")
            .iter()
            .flat_map(|s| s.iter())
            .cloned()
            .collect::<Vec<_>>();
        let (dm_name, dm_uuid) = format_backstore_ids(block_mgr.pool_uuid(), CacheRole::OriginSub);
        let ld = LinearDev::setup(dm, &dm_name, Some(&dm_uuid), map_to_dm(&segments))?;
        Ok(DataTier {
               block_mgr,
               dm_device: ld,
               segments,
               next: Sectors(0),
           })
    }

    /// Add the given paths to self. Return UUIDs of the new blockdevs
    /// corresponding to the specified paths.
    /// WARNING: metadata changing event
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
        let coalesced = coalesce_blkdevsegs(&self.segments, &segments);

        self.dm_device.set_table(dm, map_to_dm(&coalesced))?;

        self.segments = coalesced;

        Ok(uuids)
    }

    /// All the sectors available to this device
    pub fn capacity(&self) -> Sectors {
        let size = self.dm_device.size();
        assert_eq!(self.segments
                       .iter()
                       .map(|x| x.segment.length)
                       .sum::<Sectors>(),
                   size);
        size
    }

    /// The total size of all the blockdevs combined
    pub fn current_capacity(&self) -> Sectors {
        let size = self.block_mgr.current_capacity();
        assert_eq!(size - self.metadata_size(), self.capacity());
        size
    }

    /// Number of sectors unused
    pub fn available(&self) -> Sectors {
        self.capacity() - self.next
    }

    /// The number of sectors used for metadata by all the blockdevs
    pub fn metadata_size(&self) -> Sectors {
        self.block_mgr.metadata_size()
    }

    /// Allocate requested chunks from device.
    /// Returns None if it is not possible to satisfy the request.
    /// Each segment allocated is contiguous with its neighbors in the return
    /// vector.
    /// WARNING: All this must change when it becomes possible to return
    /// sectors to the store.
    /// WARNING: metadata changing event
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
pub struct Backstore {
    data_tier: DataTier,
}

impl Backstore {
    /// Make a Backstore object from blockdevs that already belong to Stratis.
    /// WARNING: metadata changing event
    pub fn setup(dm: &DM,
                 pool_uuid: PoolUuid,
                 backstore_save: &BackstoreSave,
                 devnodes: &HashMap<Device, PathBuf>,
                 last_update_time: Option<DateTime<Utc>>)
                 -> EngineResult<Backstore> {
        let blockdevs = get_blockdevs(pool_uuid, backstore_save, devnodes)?;
        let block_mgr = BlockDevMgr::new(pool_uuid, blockdevs, last_update_time);
        Ok(Backstore {
               data_tier: DataTier::setup(dm,
                                          block_mgr,
                                          &backstore_save.segments,
                                          backstore_save.next)?,
           })
    }

    /// Initialize a Backstore object, by initializing the specified devs.
    /// WARNING: metadata changing event
    pub fn initialize(dm: &DM,
                      pool_uuid: PoolUuid,
                      paths: &[&Path],
                      mda_size: Sectors,
                      force: bool)
                      -> EngineResult<Backstore> {
        Ok(Backstore {
               data_tier: DataTier::new(dm,
                                        BlockDevMgr::initialize(pool_uuid,
                                                                paths,
                                                                mda_size,
                                                                force)?)?,
           })
    }

    /// Add the given paths to self. Return UUIDs of the new blockdevs
    /// corresponding to the specified paths.
    /// WARNING: metadata changing event
    pub fn add(&mut self, dm: &DM, paths: &[&Path], force: bool) -> EngineResult<Vec<DevUuid>> {
        self.data_tier.add(dm, paths, force)
    }

    /// Allocate space from the underlying device.
    /// WARNING: metadata changing event
    pub fn alloc_space(&mut self, sizes: &[Sectors]) -> Option<Vec<Vec<(Sectors, Sectors)>>> {
        self.data_tier.alloc_space(sizes)
    }

    /// Return a reference to the blockdevs that form the basis of the
    /// backstore.
    pub fn blockdevs(&self) -> Vec<(DevUuid, &BlockDev)> {
        self.data_tier.block_mgr.blockdevs()
    }

    /// The current capacity of all the blockdevs that make up the backstore.
    pub fn current_capacity(&self) -> Sectors {
        self.data_tier.current_capacity()
    }

    /// Destroy the entire store.
    pub fn destroy(self, dm: &DM) -> EngineResult<()> {
        self.data_tier.destroy(dm)
    }

    /// Return the device that this tier is currently using.
    /// WARNING: This may change it the backstore switches between its
    /// cache and its non-cache incarnations, among other reasons.
    pub fn device(&self) -> Device {
        self.data_tier.dm_device.device()
    }


    /// Lookup an immutable blockdev by its Stratis UUID.
    pub fn get_blockdev_by_uuid(&self, uuid: DevUuid) -> Option<&BlockDev> {
        self.data_tier.block_mgr.get_blockdev_by_uuid(uuid)
    }

    /// Lookup a mutable blockdev by its Stratis UUID.
    pub fn get_mut_blockdev_by_uuid(&mut self, uuid: DevUuid) -> Option<&mut BlockDev> {
        self.data_tier.block_mgr.get_mut_blockdev_by_uuid(uuid)
    }

    /// The number of sectors in the backstore given up to Stratis
    /// metadata.
    pub fn metadata_size(&self) -> Sectors {
        self.data_tier.metadata_size()
    }

    /// Write the given data directly to the blockdevs that make up the
    /// backstore. This action bypasses the DM device entirely,
    /// in order to allow control over which blockdevs the metadata is written
    /// to.
    pub fn save_state(&mut self, metadata: &[u8]) -> EngineResult<()> {
        self.data_tier.block_mgr.save_state(metadata)
    }
}

impl Recordable<BackstoreSave> for Backstore {
    fn record(&self) -> BackstoreSave {
        BackstoreSave {
            segments: self.data_tier.segments.record(),
            block_devs: self.data_tier.block_mgr.record(),
            next: self.data_tier.next,
        }
    }
}

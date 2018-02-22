// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle the backing store of a pool.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use devicemapper::{CacheDev, DM, Device, DmDevice, IEC, LinearDev, MIN_CACHE_BLOCK_SIZE, Sectors};

use super::super::super::engine::BlockDev;
use super::super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::super::types::{BlockDevTier, DevUuid, PoolUuid};

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
/// the DM device.
#[derive(Debug)]
struct DataTier {
    /// Manages the individual block devices
    /// it is always the case block_mgr.avail_space() == 0.
    block_mgr: BlockDevMgr,
    /// The list of segments granted by block_mgr and used by dm_device
    /// It is always the case that block_mgr.avail_space() == 0, i.e., all
    /// available space in block_mgr is allocated to the DM device.
    segments: Vec<BlkDevSegment>,
}

impl DataTier {
    /// Setup a previously existing data layer from the block_mgr and
    /// previously allocated segments.
    ///
    /// Returns the DataTier and the linear DM device that was created during
    /// setup.
    ///
    /// WARNING: metadata changing event
    pub fn setup(dm: &DM,
                 block_mgr: BlockDevMgr,
                 segments: &[(DevUuid, Sectors, Sectors)])
                 -> EngineResult<(DataTier, LinearDev)> {
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

        Ok((DataTier {
                block_mgr,
                segments,
            },
            ld))
    }


    /// Setup a new DataTier struct from the block_mgr.
    ///
    /// Returns the DataTier and the linear device that was created.
    ///
    /// WARNING: metadata changing event
    pub fn new(dm: &DM, mut block_mgr: BlockDevMgr) -> EngineResult<(DataTier, LinearDev)> {
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
        Ok((DataTier {
                block_mgr,
                segments,
            },
            ld))
    }

    /// Add the given paths to self. Return UUIDs of the new blockdevs
    /// corresponding to the specified paths.
    /// WARNING: metadata changing event
    pub fn add(&mut self,
               dm: &DM,
               cache: Option<&mut CacheDev>,
               linear: Option<&mut LinearDev>,
               paths: &[&Path],
               force: bool)
               -> EngineResult<Vec<DevUuid>> {
        // These are here so that if invariant is false, the method fails
        // before allocating the segments from the block_mgr.
        // These two statements combined are equivalent to
        // cache.is_some() XOR linear.is_some(), but they may be clearer and
        // Rust does not seem to have a boolean XOR operator, anyway.
        assert!(!(cache.is_some() && linear.is_some()));
        assert!(!(cache.is_none() && linear.is_none()));

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
        let table = map_to_dm(&coalesced);

        match (cache, linear) {
            (Some(cache), None) => cache.set_origin_table(dm, table),
            (None, Some(linear)) => linear.set_table(dm, table),
            _ => panic!("see assertions at top of method"),
        }?;

        self.segments = coalesced;

        Ok(uuids)
    }

    /// All the sectors available to this device
    pub fn capacity(&self) -> Sectors {
        self.segments
            .iter()
            .map(|x| x.segment.length)
            .sum::<Sectors>()
    }

    /// The total size of all the blockdevs combined
    pub fn current_capacity(&self) -> Sectors {
        let size = self.block_mgr.current_capacity();
        assert_eq!(size - self.metadata_size(), self.capacity());
        size
    }

    /// The number of sectors used for metadata by all the blockdevs
    pub fn metadata_size(&self) -> Sectors {
        self.block_mgr.metadata_size()
    }

    /// Destroy the store. Wipe its blockdevs.
    pub fn destroy(self) -> EngineResult<()> {
        self.block_mgr.destroy_all()
    }

    /// Save the given state to the devices. This action bypasses the DM
    /// device entirely.
    pub fn save_state(&mut self, metadata: &[u8]) -> EngineResult<()> {
        self.block_mgr.save_state(metadata)
    }

    /// Lookup an immutable blockdev by its Stratis UUID.
    pub fn get_blockdev_by_uuid(&self, uuid: DevUuid) -> Option<(BlockDevTier, &BlockDev)> {
        self.block_mgr
            .get_blockdev_by_uuid(uuid)
            .and_then(|bd| Some((BlockDevTier::Data, bd)))
    }

    /// Lookup a mutable blockdev by its Stratis UUID.
    pub fn get_mut_blockdev_by_uuid(&mut self,
                                    uuid: DevUuid)
                                    -> Option<(BlockDevTier, &mut BlockDev)> {
        self.block_mgr
            .get_mut_blockdev_by_uuid(uuid)
            .and_then(|bd| Some((BlockDevTier::Data, bd)))
    }

    /// Get the blockdevs belonging to this tier
    pub fn blockdevs(&self) -> Vec<(DevUuid, &BlockDev)> {
        self.block_mgr.blockdevs()
    }
}

/// Handles the cache devices.
#[derive(Debug)]
struct CacheTier {
    /// Manages the individual block devices
    block_mgr: BlockDevMgr,
    /// The list of segments granted by block_mgr and used by the metadata
    /// device.
    meta_segments: Vec<BlkDevSegment>,
    /// The list of segments granted by block_mgr and used by the cache
    /// device.
    cache_segments: Vec<BlkDevSegment>,
}

impl CacheTier {
    /// Destroy the tier. Wipe its blockdevs.
    pub fn destroy(self) -> EngineResult<()> {
        self.block_mgr.destroy_all()
    }

    /// Lookup an immutable blockdev by its Stratis UUID.
    pub fn get_blockdev_by_uuid(&self, uuid: DevUuid) -> Option<(BlockDevTier, &BlockDev)> {
        self.block_mgr
            .get_blockdev_by_uuid(uuid)
            .and_then(|bd| Some((BlockDevTier::Cache, bd)))
    }

    /// Lookup a mutable blockdev by its Stratis UUID.
    pub fn get_mut_blockdev_by_uuid(&mut self,
                                    uuid: DevUuid)
                                    -> Option<(BlockDevTier, &mut BlockDev)> {
        self.block_mgr
            .get_mut_blockdev_by_uuid(uuid)
            .and_then(|bd| Some((BlockDevTier::Cache, bd)))
    }

    /// Add the given paths to self. Return UUIDs of the new blockdevs
    /// corresponding to the specified paths.
    /// Adds all additional space to cache sub-device.
    /// WARNING: metadata changing event
    pub fn add(&mut self,
               _dm: &DM,
               _cache_device: &mut CacheDev,
               paths: &[&Path],
               force: bool)
               -> EngineResult<Vec<DevUuid>> {
        let uuids = self.block_mgr.add(paths, force)?;

        let avail_space = self.block_mgr.avail_space();
        let segments = self.block_mgr
            .alloc_space(&[avail_space])
            .expect("asked for exactly the space available, must get")
            .iter()
            .flat_map(|s| s.iter())
            .cloned()
            .collect::<Vec<_>>();
        let coalesced = coalesce_blkdevsegs(&self.cache_segments, &segments);
        let _table = map_to_dm(&coalesced);

        // FIXME: Set the cache table.

        self.cache_segments = coalesced;

        Ok(uuids)
    }

    /// Setup a new CacheTier struct from the block_mgr.
    ///
    /// Returns the CacheTier and the cache device that was created.
    ///
    /// WARNING: metadata changing event
    #[allow(dead_code)]
    pub fn new(dm: &DM,
               mut block_mgr: BlockDevMgr,
               origin: LinearDev)
               -> EngineResult<(CacheTier, CacheDev)> {
        let avail_space = block_mgr.avail_space();
        // TODO: check whether this should be increased
        let meta_space = Sectors(4 * IEC::Ki);

        assert!(meta_space < avail_space,
                "every block device must be at least one GiB");

        let mut segments = block_mgr
            .alloc_space(&[meta_space, avail_space - meta_space])
            .expect("asked for exactly the space available, must get");

        let cache_segments = segments.pop().expect("segments.len() == 2");
        let meta_segments = segments.pop().expect("segments.len() == 1");

        let (dm_name, dm_uuid) = format_backstore_ids(block_mgr.pool_uuid(), CacheRole::MetaSub);
        let meta = LinearDev::setup(dm, &dm_name, Some(&dm_uuid), map_to_dm(&meta_segments))?;

        let (dm_name, dm_uuid) = format_backstore_ids(block_mgr.pool_uuid(), CacheRole::CacheSub);
        let cache = LinearDev::setup(dm, &dm_name, Some(&dm_uuid), map_to_dm(&cache_segments))?;

        let (dm_name, dm_uuid) = format_backstore_ids(block_mgr.pool_uuid(), CacheRole::Cache);
        let cd = CacheDev::new(dm,
                               &dm_name,
                               Some(&dm_uuid),
                               meta,
                               cache,
                               origin,
                               MIN_CACHE_BLOCK_SIZE)?;

        Ok((CacheTier {
                block_mgr,
                meta_segments,
                cache_segments,
            },
            cd))
    }
}


/// This structure can allocate additional space to the upper layer, but it
/// cannot accept returned space. When it is extended to be able to accept
/// returned space the allocation algorithm will have to be revised.
///
/// self.linear.is_some() XOR self.cache.is_some()
/// self.cache.is_some() <=> self.cache_tier.is_some()
#[derive(Debug)]
pub struct Backstore {
    /// Coordinates handling of the blockdevs that form the base.
    data_tier: DataTier,
    /// Coordinate handling of blockdevs that back the cache. Optional, since
    /// this structure can operate without a cache.
    cache_tier: Option<CacheTier>,
    /// A linear DM device.
    linear: Option<LinearDev>,
    /// A cache DM Device.
    cache: Option<CacheDev>,
    /// Index for managing allocation from dm_device.
    next: Sectors,
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
        let (data_tier, dm_device) = DataTier::setup(dm, block_mgr, &backstore_save.segments)?;
        Ok(Backstore {
               data_tier,
               cache_tier: None,
               linear: Some(dm_device),
               cache: None,
               next: backstore_save.next,
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
        let (data_tier, dm_device) =
            DataTier::new(dm,
                          BlockDevMgr::initialize(pool_uuid, paths, mda_size, force)?)?;
        Ok(Backstore {
               data_tier,
               cache_tier: None,
               linear: Some(dm_device),
               cache: None,
               next: Sectors(0),
           })
    }

    /// Add the given paths to self. Return UUIDs of the new blockdevs
    /// corresponding to the specified paths.
    /// WARNING: metadata changing event
    pub fn add_blockdevs(&mut self,
                         dm: &DM,
                         paths: &[&Path],
                         tier: BlockDevTier,
                         force: bool)
                         -> EngineResult<Vec<DevUuid>> {
        match tier {
            BlockDevTier::Cache => {
                match self.cache_tier {
                    Some(ref mut cache_tier) => {
                        let mut cache_device =
                            self.cache
                                .as_mut()
                                .expect("cache_tier.is_some() <=> self.cache.is_some()");
                        cache_tier.add(dm, &mut cache_device, paths, force)
                    }
                    None => panic!("not ready"),
                }
            }
            BlockDevTier::Data => {
                self.data_tier
                    .add(dm, self.cache.as_mut(), self.linear.as_mut(), paths, force)
            }
        }
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

    /// Return a reference to all the devs that this pool has ownership of.
    /// This includes blockdevs, cachedevs, any device about which information
    /// may be placed on the D-Bus.
    pub fn blockdevs(&self) -> Vec<(DevUuid, &BlockDev)> {
        self.data_tier.blockdevs()
    }

    /// The current capacity of all the blockdevs in the data tier.
    pub fn datadev_current_capacity(&self) -> Sectors {
        self.data_tier.current_capacity()
    }

    /// The available number of Sectors.
    pub fn available(&self) -> Sectors {
        self.data_tier.capacity() - self.next
    }

    /// Destroy the entire store.
    pub fn destroy(self, dm: &DM) -> EngineResult<()> {
        match self.cache {
            Some(cache) => {
                cache.teardown(dm)?;
                self.cache_tier
                    .expect("if dm_device is cache, cache tier exists")
                    .destroy()?;
            }
            None => {
                self.linear
                    .expect("self.cache.is_none()")
                    .teardown(dm)?;
            }
        };
        self.data_tier.destroy()
    }

    /// Teardown the store, i.e., the DM devices.
    pub fn teardown(self, dm: &DM) -> EngineResult<()> {
        self.dm_device.teardown(dm).map_err(|e| e.into())
    }

    /// Return the device that this tier is currently using.
    /// WARNING: This may change it the backstore switches between its
    /// cache and its non-cache incarnations, among other reasons.
    pub fn device(&self) -> Device {
        self.cache
            .as_ref()
            .map(|d| d.device())
            .or_else(|| self.linear.as_ref().map(|d| d.device()))
            .expect("must be one or the other")
    }

    /// Lookup an immutable blockdev by its Stratis UUID.
    pub fn get_blockdev_by_uuid(&self, uuid: DevUuid) -> Option<(BlockDevTier, &BlockDev)> {
        self.data_tier
            .get_blockdev_by_uuid(uuid)
            .or_else(|| {
                         self.cache_tier
                             .as_ref()
                             .and_then(|c| c.get_blockdev_by_uuid(uuid))
                     })
    }

    /// Lookup a mutable blockdev by its Stratis UUID.
    pub fn get_mut_blockdev_by_uuid(&mut self,
                                    uuid: DevUuid)
                                    -> Option<(BlockDevTier, &mut BlockDev)> {
        let cache_tier = &mut self.cache_tier;
        self.data_tier
            .get_mut_blockdev_by_uuid(uuid)
            .or_else(move || {
                         cache_tier
                             .as_mut()
                             .and_then(|c| c.get_mut_blockdev_by_uuid(uuid))
                     })
    }

    /// The number of sectors in the backstore given up to Stratis
    /// metadata on devices in the data tier.
    pub fn datadev_metadata_size(&self) -> Sectors {
        self.data_tier.metadata_size()
    }

    /// Write the given data to the data tier's devices.
    pub fn datadev_save_state(&mut self, metadata: &[u8]) -> EngineResult<()> {
        self.data_tier.save_state(metadata)
    }
}

impl Recordable<BackstoreSave> for Backstore {
    fn record(&self) -> BackstoreSave {
        BackstoreSave {
            segments: self.data_tier.segments.record(),
            block_devs: self.data_tier.block_mgr.record(),
            next: self.next,
        }
    }
}

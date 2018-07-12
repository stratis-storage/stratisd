// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle the backing store of a pool.

use std::path::Path;

use devicemapper::{CacheDev, DmDevice, LinearDev, Sectors};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::types::{BlockDevTier, DevUuid, PoolUuid};

use super::super::dm::get_dm;
use super::super::dmnames::{format_backstore_ids, CacheRole};

use super::blockdev::StratBlockDev;
use super::blockdevmgr::{coalesce_blkdevsegs, map_to_dm, BlkDevSegment, BlockDevMgr, Segment};

/// Handles the lowest level, base layer of this tier.
/// The dm_device organizes all block devs into a single linear allocation
/// pool. This structure can allocate additional space to the upper layer,
/// but it cannot accept returned space. When it is extended to be able to
/// accept returned space the allocation algorithm will have to be revised.
/// All available sectors on blockdevs in the manager are allocated to
/// the DM device.
#[derive(Debug)]
pub struct DataTier {
    /// Manages the individual block devices
    /// it is always the case block_mgr.avail_space() == 0.
    pub block_mgr: BlockDevMgr,
    /// The list of segments granted by block_mgr and used by dm_device
    /// It is always the case that block_mgr.avail_space() == 0, i.e., all
    /// available space in block_mgr is allocated to the DM device.
    pub segments: Vec<BlkDevSegment>,
    /// Index for managing allocation from dm_device.
    pub next: Sectors,
}

impl DataTier {
    /// Setup a previously existing data layer from the block_mgr and
    /// previously allocated segments.
    ///
    /// next is the location of the next sector that can be allocated.
    ///
    /// Returns the DataTier and the linear DM device that was created during
    /// setup.
    pub fn setup(
        pool_uuid: PoolUuid,
        block_mgr: BlockDevMgr,
        segments: &[(DevUuid, Sectors, Sectors)],
        next: Sectors,
    ) -> StratisResult<(DataTier, LinearDev)> {
        if block_mgr.avail_space() != Sectors(0) {
            let err_msg = format!(
                "{} unallocated to device; probable metadata corruption",
                block_mgr.avail_space()
            );
            return Err(StratisError::Engine(ErrorEnum::Error, err_msg));
        }

        let uuid_to_devno = block_mgr.uuid_to_devno();
        let mapper = |triple: &(DevUuid, Sectors, Sectors)| -> StratisResult<BlkDevSegment> {
            let device = uuid_to_devno(triple.0).ok_or_else(|| {
                StratisError::Engine(
                    ErrorEnum::NotFound,
                    format!("missing device for UUUD {:?}", &triple.0),
                )
            })?;
            Ok(BlkDevSegment::new(
                triple.0,
                Segment::new(device, triple.1, triple.2),
            ))
        };
        let segments = segments
            .iter()
            .map(&mapper)
            .collect::<StratisResult<Vec<_>>>()?;

        let (dm_name, dm_uuid) = format_backstore_ids(pool_uuid, CacheRole::OriginSub);
        let ld = LinearDev::setup(get_dm(), &dm_name, Some(&dm_uuid), map_to_dm(&segments))?;

        Ok((
            DataTier {
                block_mgr,
                segments,
                next,
            },
            ld,
        ))
    }

    /// Setup a new DataTier struct from the block_mgr.
    ///
    /// Returns the DataTier and the linear device that was created.
    ///
    /// WARNING: metadata changing event
    pub fn new(
        pool_uuid: PoolUuid,
        mut block_mgr: BlockDevMgr,
    ) -> StratisResult<(DataTier, LinearDev)> {
        let avail_space = block_mgr.avail_space();
        let segments = block_mgr
            .alloc_space(&[avail_space])
            .expect("asked for exactly the space available, must get")
            .iter()
            .flat_map(|s| s.iter())
            .cloned()
            .collect::<Vec<_>>();
        let (dm_name, dm_uuid) = format_backstore_ids(pool_uuid, CacheRole::OriginSub);
        let ld = LinearDev::setup(get_dm(), &dm_name, Some(&dm_uuid), map_to_dm(&segments))?;
        Ok((
            DataTier {
                block_mgr,
                segments,
                next: Sectors(0),
            },
            ld,
        ))
    }

    /// Add the given paths to self. Return UUIDs of the new blockdevs
    /// corresponding to the specified paths.
    /// WARNING: metadata changing event
    pub fn add(
        &mut self,
        pool_uuid: PoolUuid,
        cache: Option<&mut CacheDev>,
        linear: Option<&mut LinearDev>,
        paths: &[&Path],
        force: bool,
    ) -> StratisResult<Vec<DevUuid>> {
        // These are here so that if invariant is false, the method fails
        // before allocating the segments from the block_mgr.
        // These two statements combined are equivalent to
        // cache.is_some() XOR linear.is_some(), but they may be clearer and
        // Rust does not seem to have a boolean XOR operator, anyway.
        assert!(!(cache.is_some() && linear.is_some()));
        assert!(!(cache.is_none() && linear.is_none()));

        let uuids = self.block_mgr.add(pool_uuid, paths, force)?;

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
            (Some(cache), None) => {
                cache.set_origin_table(get_dm(), table)?;
                cache.resume(get_dm())
            }
            (None, Some(linear)) => {
                linear.set_table(get_dm(), table)?;
                linear.resume(get_dm())
            }
            _ => panic!("see assertions at top of method"),
        }?;

        self.segments = coalesced;

        Ok(uuids)
    }

    /// The number of Sectors that remain to be allocated.
    pub fn available(&self) -> Sectors {
        self.capacity() - self.next
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

    /// All the sectors available to this device
    pub fn capacity(&self) -> Sectors {
        self.segments
            .iter()
            .map(|x| x.segment.length)
            .sum::<Sectors>()
    }

    /// The total size of all the blockdevs combined
    pub fn current_capacity(&self) -> Sectors {
        self.block_mgr.current_capacity()
    }

    /// The number of sectors used for metadata by all the blockdevs
    pub fn metadata_size(&self) -> Sectors {
        self.block_mgr.metadata_size()
    }

    /// Destroy the store. Wipe its blockdevs.
    pub fn destroy(self) -> StratisResult<()> {
        self.block_mgr.destroy_all()
    }

    /// Save the given state to the devices. This action bypasses the DM
    /// device entirely.
    pub fn save_state(&mut self, metadata: &[u8]) -> StratisResult<()> {
        self.block_mgr.save_state(metadata)
    }

    /// Lookup an immutable blockdev by its Stratis UUID.
    pub fn get_blockdev_by_uuid(&self, uuid: DevUuid) -> Option<(BlockDevTier, &StratBlockDev)> {
        self.block_mgr
            .get_blockdev_by_uuid(uuid)
            .and_then(|bd| Some((BlockDevTier::Data, bd)))
    }

    /// Lookup a mutable blockdev by its Stratis UUID.
    pub fn get_mut_blockdev_by_uuid(
        &mut self,
        uuid: DevUuid,
    ) -> Option<(BlockDevTier, &mut StratBlockDev)> {
        self.block_mgr
            .get_mut_blockdev_by_uuid(uuid)
            .and_then(|bd| Some((BlockDevTier::Data, bd)))
    }

    /// Get the blockdevs belonging to this tier
    pub fn blockdevs(&self) -> Vec<(DevUuid, &StratBlockDev)> {
        self.block_mgr.blockdevs()
    }

    /// Assert things that should always hold true of a DataTier
    #[allow(dead_code)]
    fn invariant(&self) -> () {
        assert_eq!(
            self.block_mgr.current_capacity() - self.metadata_size(),
            self.capacity()
        )
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle the backing store of a pool.

use std::path::Path;

use devicemapper::{CacheDev, LinearDev, Sectors};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::types::{BlockDevTier, DevUuid, PoolUuid};

use super::super::dm::get_dm;
use super::super::dmnames::{format_backstore_ids, CacheRole};

use super::blockdev::StratBlockDev;
use super::blockdevmgr::{map_to_dm, BlkDevSegment, BlockDevMgr, Segment};

/// Handles the lowest level, base layer of this tier.
/// This structure can allocate additional space to the upper layer,
/// but it cannot accept returned space. When it is extended to be able to
/// accept returned space the allocation algorithm will have to be revised.
#[derive(Debug)]
pub struct DataTier {
    /// Manages the individual block devices.
    pub block_mgr: BlockDevMgr,
    /// The list of segments granted by block_mgr to the cap device.
    pub segments: Vec<BlkDevSegment>,
}

impl DataTier {
    /// Setup a previously existing data layer from the block_mgr and
    /// previously allocated segments.
    ///
    /// Returns the DataTier and the linear DM device that was created during
    /// setup.
    pub fn setup(
        pool_uuid: PoolUuid,
        block_mgr: BlockDevMgr,
        segments: &[(DevUuid, Sectors, Sectors)],
    ) -> StratisResult<(DataTier, LinearDev)> {
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
            },
            ld,
        ))
    }

    /// Setup a new DataTier struct from the block_mgr.
    ///
    /// Returns the DataTier and the linear device that was created.
    pub fn new(block_mgr: BlockDevMgr) -> StratisResult<DataTier> {
        Ok(DataTier {
            block_mgr,
            segments: vec![],
        })
    }

    /// Add the given paths to self. Return UUIDs of the new blockdevs
    /// corresponding to the specified paths.
    /// WARNING: metadata changing event
    pub fn add(
        &mut self,
        pool_uuid: PoolUuid,
        paths: &[&Path],
        force: bool,
    ) -> StratisResult<Vec<DevUuid>> {
        Ok(self.block_mgr.add(pool_uuid, paths, force)?)
    }

    /// Allocate requested segments and add to the end of the linear device.
    /// Return None if the request can not be satisfied.
    /// Precondition: This method is called only when
    pub fn alloc_segments(
        &mut self,
        _request: Sectors,
        cache: Option<&mut CacheDev>,
        linear: Option<&mut LinearDev>,
    ) -> StratisResult<()> {
        assert!(!(cache.is_some() && linear.is_some()));
        unimplemented!();
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

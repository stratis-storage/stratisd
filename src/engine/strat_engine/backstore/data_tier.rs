// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle the backing store of a pool.

#[cfg(test)]
use std::collections::HashSet;

use devicemapper::Sectors;

use crate::{
    engine::{
        strat_engine::{
            backstore::{
                blockdev::{v1::StratBlockDev, InternalBlockDev},
                blockdevmgr::BlockDevMgr,
                devices::UnownedDevices,
                shared::{metadata_to_segment, AllocatedAbove, BlkDevSegment, BlockDevPartition},
            },
            serde_structs::{BaseDevSave, BlockDevSave, DataTierSave, Recordable},
            types::BDARecordResult,
        },
        types::{BlockDevTier, DevUuid, Name, PoolUuid},
    },
    stratis::StratisResult,
};

/// Handles the lowest level, base layer of this tier.
#[derive(Debug)]
pub struct DataTier {
    /// Manages the individual block devices
    pub(super) block_mgr: BlockDevMgr,
    /// The list of segments granted by block_mgr and used by dm_device
    pub(super) segments: AllocatedAbove,
}

impl DataTier {
    /// Setup a previously existing data layer from the block_mgr and
    /// previously allocated segments.
    pub fn setup(
        block_mgr: BlockDevMgr,
        data_tier_save: &DataTierSave,
    ) -> BDARecordResult<DataTier> {
        let uuid_to_devno = block_mgr.uuid_to_devno();
        let mapper = |ld: &BaseDevSave| -> StratisResult<BlkDevSegment> {
            metadata_to_segment(&uuid_to_devno, ld)
        };
        let segments = match data_tier_save.blockdev.allocs[0]
            .iter()
            .map(&mapper)
            .collect::<StratisResult<Vec<_>>>()
        {
            Ok(s) => AllocatedAbove { inner: s },
            Err(e) => return Err((e, block_mgr.into_bdas())),
        };

        Ok(DataTier {
            block_mgr,
            segments,
        })
    }

    /// Setup a new DataTier struct from the block_mgr.
    ///
    /// Initially 0 segments are allocated.
    ///
    /// WARNING: metadata changing event
    pub fn new(block_mgr: BlockDevMgr) -> DataTier {
        DataTier {
            block_mgr,
            segments: AllocatedAbove { inner: vec![] },
        }
    }

    /// Add the given paths to self. Return UUIDs of the new blockdevs
    /// corresponding to the specified paths.
    /// WARNING: metadata changing event
    pub fn add(
        &mut self,
        pool_name: Name,
        pool_uuid: PoolUuid,
        devices: UnownedDevices,
        sector_size: Option<u32>,
    ) -> StratisResult<Vec<DevUuid>> {
        self.block_mgr
            .add(pool_name, pool_uuid, devices, sector_size)
    }

    /// Allocate a region for all sector size requests from unallocated segments in
    /// block devices belonging to the data tier. Return true if requested
    /// amount or more was allocated, otherwise, false.
    pub fn alloc(&mut self, requests: &[Sectors]) -> bool {
        self.block_mgr
            .alloc(requests)
            .map(|segments| {
                self.segments.coalesce_blkdevsegs(
                    &segments
                        .iter()
                        .flat_map(|s| s.iter())
                        .cloned()
                        .collect::<Vec<_>>(),
                );
                true
            })
            .unwrap_or(false)
    }

    /// The sum of the lengths of all the sectors that have been mapped to an
    /// upper device.
    pub fn allocated(&self) -> Sectors {
        self.segments.size()
    }

    /// The total size of all the blockdevs combined
    pub fn size(&self) -> Sectors {
        self.block_mgr.size()
    }

    /// The number of sectors used for metadata by all the blockdevs
    pub fn metadata_size(&self) -> Sectors {
        self.block_mgr.metadata_size()
    }

    /// The total usable size of all the blockdevs combined
    pub fn usable_size(&self) -> Sectors {
        self.size() - self.metadata_size()
    }

    /// Destroy the store. Wipe its blockdevs.
    pub fn destroy(&mut self) -> StratisResult<()> {
        self.block_mgr.destroy_all()
    }

    /// Save the given state to the devices. This action bypasses the DM
    /// device entirely.
    pub fn save_state(&mut self, metadata: &[u8]) -> StratisResult<()> {
        self.block_mgr.save_state(metadata)
    }

    pub fn load_state(&self) -> StratisResult<Vec<u8>> {
        self.block_mgr.load_state()
    }

    /// Lookup an immutable blockdev by its Stratis UUID.
    pub fn get_blockdev_by_uuid(&self, uuid: DevUuid) -> Option<(BlockDevTier, &StratBlockDev)> {
        self.block_mgr
            .get_blockdev_by_uuid(uuid)
            .map(|bd| (BlockDevTier::Data, bd))
    }

    /// Lookup a mutable blockdev by its Stratis UUID.
    pub fn get_mut_blockdev_by_uuid(
        &mut self,
        uuid: DevUuid,
    ) -> Option<(BlockDevTier, &mut StratBlockDev)> {
        self.block_mgr
            .get_mut_blockdev_by_uuid(uuid)
            .map(|bd| (BlockDevTier::Data, bd))
    }

    /// Get the blockdevs belonging to this tier
    pub fn blockdevs(&self) -> Vec<(DevUuid, &StratBlockDev)> {
        self.block_mgr.blockdevs()
    }

    pub fn blockdevs_mut(&mut self) -> Vec<(DevUuid, &mut StratBlockDev)> {
        self.block_mgr.blockdevs_mut()
    }

    pub fn grow(&mut self, dev: DevUuid) -> StratisResult<bool> {
        self.block_mgr.grow(dev)
    }

    /// Return the partition of the block devs that are in use and those
    /// that are not.
    pub fn partition_by_use(&self) -> BlockDevPartition<'_> {
        let blockdevs = self.block_mgr.blockdevs();
        let (used, unused) = blockdevs.iter().partition(|(_, bd)| bd.in_use());
        BlockDevPartition { used, unused }
    }

    #[cfg(test)]
    pub fn invariant(&self) {
        let allocated_uuids = self.segments.uuids();
        let in_use_uuids = self
            .block_mgr
            .blockdevs()
            .iter()
            .filter(|(_, bd)| bd.in_use())
            .map(|(u, _)| *u)
            .collect::<HashSet<_>>();
        assert_eq!(allocated_uuids, in_use_uuids);
    }
}

impl Recordable<DataTierSave> for DataTier {
    fn record(&self) -> DataTierSave {
        DataTierSave {
            blockdev: BlockDevSave {
                allocs: vec![self.segments.record()],
                devs: self.block_mgr.record(),
            },
        }
    }
}

#[cfg(test)]
mod tests {

    use std::path::Path;

    use crate::engine::strat_engine::{
        backstore::devices::{ProcessedPathInfos, UnownedDevices},
        metadata::MDADataSize,
        tests::{loopbacked, real},
    };

    use super::*;

    fn get_devices(paths: &[&Path]) -> StratisResult<UnownedDevices> {
        ProcessedPathInfos::try_from(paths)
            .map(|ps| ps.unpack())
            .map(|(sds, uds)| {
                sds.error_on_not_empty().unwrap();
                uds
            })
    }

    /// Put the data tier through some paces. Make it, alloc a small amount,
    /// add some more blockdevs, allocate enough that the newly added blockdevs
    /// must be allocated from for success.
    fn test_add_and_alloc(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let pool_uuid = PoolUuid::new_v4();
        let pool_name = Name::new("pool_name".to_string());

        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let devices1 = get_devices(paths1).unwrap();
        let devices2 = get_devices(paths2).unwrap();

        let mgr = BlockDevMgr::initialize(
            pool_name.clone(),
            pool_uuid,
            devices1,
            MDADataSize::default(),
            None,
            None,
        )
        .unwrap();

        let mut data_tier = DataTier::new(mgr);
        data_tier.invariant();

        // A data_tier w/ some devices but nothing allocated
        let mut size = data_tier.size();
        let mut allocated = data_tier.allocated();
        assert_eq!(allocated, Sectors(0));
        assert!(size != Sectors(0));

        let last_request_amount = size;

        let request_amount = data_tier.block_mgr.avail_space() / 2usize;
        assert!(request_amount != Sectors(0));

        assert!(data_tier.alloc(&[request_amount]));
        data_tier.invariant();

        // A data tier w/ some amount allocated
        assert!(data_tier.allocated() >= request_amount);
        assert_eq!(data_tier.size(), size);
        allocated = data_tier.allocated();

        data_tier.add(pool_name, pool_uuid, devices2, None).unwrap();
        data_tier.invariant();

        // A data tier w/ additional blockdevs added
        assert!(data_tier.size() > size);
        assert_eq!(data_tier.allocated(), allocated);
        assert_eq!(paths.len(), data_tier.blockdevs().len());
        size = data_tier.size();

        // Allocate enough to get into the newly added block devices
        assert!(data_tier.alloc(&[last_request_amount]));
        data_tier.invariant();

        assert!(data_tier.allocated() >= request_amount + last_request_amount);
        assert_eq!(data_tier.size(), size);

        data_tier.destroy().unwrap();
    }

    #[test]
    fn loop_test_add_and_alloc() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_add_and_alloc,
        );
    }

    #[test]
    fn real_test_add_and_alloc() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_add_and_alloc,
        );
    }
}

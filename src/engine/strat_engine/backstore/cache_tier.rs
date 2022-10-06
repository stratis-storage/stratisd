// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle the backing store of a pool.

use devicemapper::{Sectors, IEC, SECTOR_SIZE};

use crate::{
    engine::{
        strat_engine::{
            backstore::{
                blockdev::StratBlockDev,
                blockdevmgr::{BlkDevSegment, BlockDevMgr},
                devices::UnownedDevices,
                shared::{coalesce_blkdevsegs, metadata_to_segment},
            },
            serde_structs::{BaseDevSave, BlockDevSave, CacheTierSave, Recordable},
        },
        types::{BlockDevTier, DevUuid, PoolUuid},
    },
    stratis::{StratisError, StratisResult},
};

/// This is a temporary maximum cache size. In the future it will be possible
/// to dynamically increase the cache size beyond this limit. When this is
/// achieved this constant definition should be removed. This choice of a
/// maximum cache size is a function of the current values for the cache block
/// size, 2 Ki-sectors, and the current value for the metadata sub-device size,
/// 1 Mi-sectors.
const MAX_CACHE_SIZE: Sectors = Sectors(32 * IEC::Ti / SECTOR_SIZE as u64);

/// Handles the cache devices.
#[derive(Debug)]
pub struct CacheTier {
    /// Manages the individual block devices
    pub block_mgr: BlockDevMgr,
    /// The list of segments granted by block_mgr and used by the cache
    /// device.
    pub cache_segments: Vec<BlkDevSegment>,
    /// The list of segments granted by block_mgr and used by the metadata
    /// device.
    pub meta_segments: Vec<BlkDevSegment>,
}

impl CacheTier {
    /// Setup a previously existing cache layer from the block_mgr and
    /// previously allocated segments.
    pub fn setup(
        block_mgr: BlockDevMgr,
        cache_tier_save: &CacheTierSave,
    ) -> StratisResult<CacheTier> {
        if block_mgr.avail_space() != Sectors(0) {
            let err_msg = format!(
                "{} unallocated to device; probable metadata corruption",
                block_mgr.avail_space()
            );
            return Err(StratisError::Msg(err_msg));
        }

        let uuid_to_devno = block_mgr.uuid_to_devno();
        let mapper = |ld: &BaseDevSave| -> StratisResult<BlkDevSegment> {
            metadata_to_segment(&uuid_to_devno, ld)
        };

        let meta_segments = cache_tier_save.blockdev.allocs[1]
            .iter()
            .map(&mapper)
            .collect::<StratisResult<Vec<_>>>()?;

        let cache_segments = cache_tier_save.blockdev.allocs[0]
            .iter()
            .map(&mapper)
            .collect::<StratisResult<Vec<_>>>()?;

        Ok(CacheTier {
            block_mgr,
            cache_segments,
            meta_segments,
        })
    }

    /// Add the given paths to self. Return UUIDs of the new blockdevs
    /// corresponding to the specified paths and a pair of Boolean values.
    /// The first is true if the cache sub-device's segments were changed,
    /// the second is true if the meta sub-device's segments were changed.
    /// Adds all additional space to cache sub-device.
    /// WARNING: metadata changing event
    ///
    /// Return an error if the addition of the cachedevs would result in a
    /// cache with a cache sub-device size greater than 32 TiB.
    ///
    // FIXME: That all segments on the newly added device are added to the
    // cache sub-device and none to the meta sub-device could lead to failure.
    // Presumably, the size required for the meta sub-device varies directly
    // with the size of cache sub-device.
    pub fn add(
        &mut self,
        pool_uuid: PoolUuid,
        devices: UnownedDevices,
    ) -> StratisResult<(Vec<DevUuid>, (bool, bool))> {
        let uuids = self.block_mgr.add(pool_uuid, devices)?;

        let avail_space = self.block_mgr.avail_space();

        // FIXME: This check will become unnecessary when cache metadata device
        // can be increased dynamically.
        if avail_space
            + self
                .cache_segments
                .iter()
                .map(|x| x.segment.length)
                .sum::<Sectors>()
            > MAX_CACHE_SIZE
        {
            self.block_mgr.remove_blockdevs(&uuids)?;
            return Err(StratisError::Msg(format!(
                "The size of the cache sub-device may not exceed {}",
                MAX_CACHE_SIZE
            )));
        }

        let trans = self
            .block_mgr
            .request_space(&[avail_space])?
            .expect("asked for exactly the space available, must get");
        let segments = trans.get_blockdevmgr();
        if let Err(e) = self.block_mgr.commit_space(trans) {
            self.block_mgr.remove_blockdevs(&uuids)?;
            return Err(StratisError::Msg(format!(
                "Failed to commit metadata changes: {}",
                e
            )));
        }
        self.cache_segments = coalesce_blkdevsegs(&self.cache_segments, &segments);

        Ok((uuids, (true, false)))
    }

    /// Setup a new CacheTier struct from the block_mgr.
    ///
    /// Returns an error if the block devices passed would make the cache
    /// sub-device too big.
    ///
    /// WARNING: metadata changing event
    pub fn new(mut block_mgr: BlockDevMgr) -> StratisResult<CacheTier> {
        let avail_space = block_mgr.avail_space();

        // FIXME: Come up with a better way to choose metadata device size
        let meta_space = Sectors(IEC::Mi);

        assert!(
            meta_space < avail_space,
            "every block device must be at least one GiB"
        );

        // FIXME: This check will become unnecessary when cache metadata device
        // can be increased dynamically.
        if avail_space - meta_space > MAX_CACHE_SIZE {
            block_mgr.destroy_all()?;
            return Err(StratisError::Msg(format!(
                "The size of the cache sub-device may not exceed {}",
                MAX_CACHE_SIZE
            )));
        }

        let trans = block_mgr
            .request_space(&[meta_space, avail_space - meta_space])?
            .expect("asked for exactly the space available, must get");
        let meta_segments = trans.get_segs_for_req(0).expect("segments.len() == 2");
        let cache_segments = trans.get_segs_for_req(1).expect("segments.len() == 2");
        if let Err(e) = block_mgr.commit_space(trans) {
            block_mgr.destroy_all()?;
            return Err(StratisError::Msg(format!(
                "Failed to commit metadata changes: {}",
                e
            )));
        }

        Ok(CacheTier {
            block_mgr,
            cache_segments,
            meta_segments,
        })
    }

    /// Destroy the tier. Wipe its blockdevs.
    pub fn destroy(&mut self) -> StratisResult<()> {
        self.block_mgr.destroy_all()
    }

    /// Get all the blockdevs belonging to this tier.
    pub fn blockdevs(&self) -> Vec<(DevUuid, &StratBlockDev)> {
        self.block_mgr.blockdevs()
    }

    pub fn blockdevs_mut(&mut self) -> Vec<(DevUuid, &mut StratBlockDev)> {
        self.block_mgr.blockdevs_mut()
    }

    /// Lookup an immutable blockdev by its Stratis UUID.
    pub fn get_blockdev_by_uuid(&self, uuid: DevUuid) -> Option<(BlockDevTier, &StratBlockDev)> {
        self.block_mgr
            .get_blockdev_by_uuid(uuid)
            .map(|bd| (BlockDevTier::Cache, bd))
    }

    /// Lookup a mutable blockdev by its Stratis UUID.
    pub fn get_mut_blockdev_by_uuid(
        &mut self,
        uuid: DevUuid,
    ) -> Option<(BlockDevTier, &mut StratBlockDev)> {
        self.block_mgr
            .get_mut_blockdev_by_uuid(uuid)
            .map(|bd| (BlockDevTier::Cache, bd))
    }
}

impl Recordable<CacheTierSave> for CacheTier {
    fn record(&self) -> CacheTierSave {
        CacheTierSave {
            blockdev: BlockDevSave {
                allocs: vec![self.cache_segments.record(), self.meta_segments.record()],
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

    /// Do basic testing of the cache. Make a new cache and test some
    /// expected properties, then add some additional blockdevs and test
    /// some more properties.
    fn cache_test_add(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let pool_uuid = PoolUuid::new_v4();

        let devices1 = get_devices(paths1).unwrap();
        let devices2 = get_devices(paths2).unwrap();

        let mgr =
            BlockDevMgr::initialize(pool_uuid, devices1, MDADataSize::default(), None).unwrap();

        let mut cache_tier = CacheTier::new(mgr).unwrap();

        // A cache tier w/ some devices and everything promptly allocated to
        // the tier.
        let cache_metadata_size = cache_tier
            .meta_segments
            .iter()
            .map(|x| x.segment.length)
            .sum::<Sectors>();

        let mut metadata_size = cache_tier.block_mgr.metadata_size();
        let mut size = cache_tier.block_mgr.size();
        let mut allocated = cache_tier
            .cache_segments
            .iter()
            .map(|x| x.segment.length)
            .sum::<Sectors>();

        assert_eq!(cache_tier.block_mgr.avail_space(), Sectors(0));
        assert_eq!(size - metadata_size, allocated + cache_metadata_size);

        let (_, (cache, meta)) = cache_tier.add(pool_uuid, devices2).unwrap();
        // TODO: Ultimately, it should be the case that meta can be true.
        assert!(cache);
        assert!(!meta);

        assert_eq!(cache_tier.block_mgr.avail_space(), Sectors(0));
        assert!(cache_tier.block_mgr.size() > size);
        assert!(cache_tier.block_mgr.metadata_size() > metadata_size);

        metadata_size = cache_tier.block_mgr.metadata_size();
        size = cache_tier.block_mgr.size();
        allocated = cache_tier
            .cache_segments
            .iter()
            .map(|x| x.segment.length)
            .sum::<Sectors>();
        assert_eq!(size - metadata_size, allocated + cache_metadata_size);

        cache_tier.destroy().unwrap();
    }

    #[test]
    fn loop_cache_test_add() {
        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Range(2, 3, None), cache_test_add);
    }

    #[test]
    fn real_cache_test_add() {
        real::test_with_spec(&real::DeviceLimits::AtLeast(2, None, None), cache_test_add);
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle the backing store of a pool.

use std::cmp;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use devicemapper::{CacheDev, Device, DmDevice, LinearDev, Sectors, IEC, SECTOR_SIZE};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::types::{BlockDevTier, DevUuid, PoolUuid};

use super::super::device::wipe_sectors;
use super::super::dm::get_dm;
use super::super::names::{format_backstore_ids, CacheRole};
use super::super::serde_structs::{BackstoreSave, CapSave, Recordable};

use super::blockdev::StratBlockDev;
use super::blockdevmgr::{map_to_dm, BlkDevSegment, BlockDevMgr};
use super::cache_tier::CacheTier;
use super::data_tier::DataTier;
use super::metadata::MIN_MDA_SECTORS;
use super::setup::get_blockdevs;

/// Use a cache block size that the kernel docs indicate is the largest
/// typical size.
const CACHE_BLOCK_SIZE: Sectors = Sectors(2048); // 1024 KiB

/// Temporarily limit total cache size to this value.
const MAX_CACHE_SIZE: Sectors = Sectors(32 * IEC::Ti / SECTOR_SIZE as u64);

/// Make a DM cache device. If the cache device is being made new,
/// take extra steps to make it clean.
fn make_cache(
    pool_uuid: PoolUuid,
    cache_tier: &CacheTier,
    origin: LinearDev,
    new: bool,
) -> StratisResult<CacheDev> {
    let (dm_name, dm_uuid) = format_backstore_ids(pool_uuid, CacheRole::MetaSub);
    let meta = LinearDev::setup(
        get_dm(),
        &dm_name,
        Some(&dm_uuid),
        map_to_dm(&cache_tier.meta_segments),
    )?;

    if new {
        // See comment in ThinPool::new() method
        wipe_sectors(
            &meta.devnode(),
            Sectors(0),
            cmp::min(Sectors(8), meta.size()),
        )?;
    }

    let (dm_name, dm_uuid) = format_backstore_ids(pool_uuid, CacheRole::CacheSub);
    let cache = LinearDev::setup(
        get_dm(),
        &dm_name,
        Some(&dm_uuid),
        map_to_dm(&cache_tier.cache_segments),
    )?;

    let (dm_name, dm_uuid) = format_backstore_ids(pool_uuid, CacheRole::Cache);
    Ok(CacheDev::setup(
        get_dm(),
        &dm_name,
        Some(&dm_uuid),
        meta,
        cache,
        origin,
        CACHE_BLOCK_SIZE,
    )?)
}

/// This structure can allocate additional space to the upper layer, but it
/// cannot accept returned space. When it is extended to be able to accept
/// returned space the allocation algorithm will have to be revised.
#[derive(Debug)]
pub struct Backstore {
    /// A cache DM Device.
    cache: Option<CacheDev>,
    /// Coordinate handling of blockdevs that back the cache. Optional, since
    /// this structure can operate without a cache.
    cache_tier: Option<CacheTier>,
    /// Coordinates handling of the blockdevs that form the base.
    data_tier: DataTier,
    /// A linear DM device.
    linear: Option<LinearDev>,
    /// Index for managing allocation of cap device
    next: Sectors,
}

/// Limit max size of cachedev in a very crude way, until we can support
/// extending the cache meta device.
// Ignore all blkdevsegments that contain sectors that are beyond 32 TiB
// TODO: Remove the need for this function
fn constrain_max_cache_size(segs: &[BlkDevSegment]) -> &[BlkDevSegment] {
    let mut acc = Sectors(0);
    for (idx, seg) in segs.iter().enumerate() {
        acc = Sectors((*acc).saturating_add(*seg.segment.length));
        if acc > MAX_CACHE_SIZE {
            return &segs[..idx];
        }
    }

    segs
}

impl Backstore {
    /// Make a Backstore object from blockdevs that already belong to Stratis.
    /// Precondition: every device in devnodes has already been determined to
    /// belong to the pool with the specified pool_uuid.
    /// Precondition: backstore_save.cap.allocs[0].length <=
    ///       the sum of the lengths of the segments allocated
    /// to the data tier cap device.
    /// Precondition: backstore_save.data_segments is not empty. This is a
    /// consequence of the fact that metadata is saved by the pool, and if
    /// a pool exists, data has been allocated to the cap device.
    /// Postcondition:
    /// self.linear.is_some() XOR self.cache.is_some()
    /// self.cache.is_some() <=> self.cache_tier.is_some()
    pub fn setup(
        pool_uuid: PoolUuid,
        backstore_save: &BackstoreSave,
        devnodes: &HashMap<Device, PathBuf>,
        last_update_time: Option<DateTime<Utc>>,
    ) -> StratisResult<Backstore> {
        let (datadevs, cachedevs) = get_blockdevs(pool_uuid, backstore_save, devnodes)?;
        let block_mgr = BlockDevMgr::new(datadevs, last_update_time);
        let data_tier = DataTier::setup(block_mgr, &backstore_save.data_tier)?;
        let (dm_name, dm_uuid) = format_backstore_ids(pool_uuid, CacheRole::OriginSub);
        let origin = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            map_to_dm(&data_tier.segments),
        )?;

        let (cache_tier, cache, origin) = if !cachedevs.is_empty() {
            let block_mgr = BlockDevMgr::new(cachedevs, last_update_time);
            match backstore_save.cache_tier {
                Some(ref cache_tier_save) => {
                    let cache_tier = CacheTier::setup(block_mgr, &cache_tier_save)?;

                    let cache_device = make_cache(pool_uuid, &cache_tier, origin, false)?;
                    (Some(cache_tier), Some(cache_device), None)
                }
                None => {
                    let err_msg = "Cachedevs exist, but cache metdata does not exist";
                    return Err(StratisError::Engine(ErrorEnum::Error, err_msg.into()));
                }
            }
        } else {
            (None, None, Some(origin))
        };

        Ok(Backstore {
            data_tier,
            cache_tier,
            linear: origin,
            cache,
            next: backstore_save.cap.allocs[0].1,
        })
    }

    /// Initialize a Backstore object, by initializing the specified devs.
    ///
    /// Immediately after initialization a backstore has no cap device, since
    /// no segments are allocated in the data tier.
    ///
    /// WARNING: metadata changing event
    pub fn initialize(
        pool_uuid: PoolUuid,
        paths: &[&Path],
        mda_size: Sectors,
    ) -> StratisResult<Backstore> {
        let data_tier = DataTier::new(BlockDevMgr::initialize(pool_uuid, paths, mda_size)?);

        Ok(Backstore {
            data_tier,
            cache_tier: None,
            linear: None,
            cache: None,
            next: Sectors(0),
        })
    }

    /// Add cachedevs to the backstore.
    ///
    /// If the cache tier does not already exist, create it.
    ///
    /// Precondition: Must be invoked only after some space has been allocated
    /// from the backstore. This ensures that there is certainly a cap device.
    // Precondition: self.linear.is_some() XOR self.cache.is_some()
    // Postcondition: self.cache.is_some() && self.linear.is_none()
    pub fn add_cachedevs(
        &mut self,
        pool_uuid: PoolUuid,
        paths: &[&Path],
    ) -> StratisResult<Vec<DevUuid>> {
        match self.cache_tier {
            Some(ref mut cache_tier) => {
                let mut cache_device = self.cache
                    .as_mut()
                    .expect("cache_tier.is_some() <=> self.cache.is_some()");
                let (uuids, (cache_change, meta_change)) = cache_tier.add(pool_uuid, paths)?;

                if cache_change {
                    let table = map_to_dm(constrain_max_cache_size(&cache_tier.cache_segments));
                    cache_device.set_cache_table(get_dm(), table)?;
                    cache_device.resume(get_dm())?;
                }

                // NOTE: currently CacheTier::add() does not ever update the
                // meta segments. That means that this code is dead. But,
                // when CacheTier::add() is fixed, this code will become live.
                if meta_change {
                    let table = map_to_dm(&cache_tier.meta_segments);
                    cache_device.set_meta_table(get_dm(), table)?;
                    cache_device.resume(get_dm())?;
                }

                Ok(uuids)
            }
            None => {
                let bdm = BlockDevMgr::initialize(pool_uuid, paths, MIN_MDA_SECTORS)?;

                let cache_tier = CacheTier::new(bdm);

                let linear = self.linear
                    .take()
                    .expect("some space has already been allocated from the backstore => (cache_tier.is_none() <=> self.linear.is_some())");

                let cache = make_cache(pool_uuid, &cache_tier, linear, true)?;

                self.cache = Some(cache);

                let uuids = cache_tier
                    .block_mgr
                    .blockdevs()
                    .iter()
                    .map(|&(uuid, _)| uuid)
                    .collect::<Vec<_>>();

                self.cache_tier = Some(cache_tier);

                Ok(uuids)
            }
        }
    }

    /// Add datadevs to the backstore. The data tier always exists if the
    /// backstore exists at all, so there is no need to create it.
    pub fn add_datadevs(
        &mut self,
        pool_uuid: PoolUuid,
        paths: &[&Path],
    ) -> StratisResult<Vec<DevUuid>> {
        self.data_tier.add(pool_uuid, paths)
    }

    /// Extend the cap device whether it is a cache or not. Create the DM
    /// device if it does not already exist. Return an error if DM
    /// operations fail. Use all segments currently allocated in the data tier.
    fn extend_cap_device(&mut self, pool_uuid: PoolUuid) -> StratisResult<()> {
        let create = match (self.cache.as_mut(), self.linear.as_mut()) {
            (None, None) => true,
            (Some(cache), None) => {
                let table = map_to_dm(&self.data_tier.segments);
                cache.set_origin_table(get_dm(), table)?;
                cache.resume(get_dm())?;
                false
            }
            (None, Some(linear)) => {
                let table = map_to_dm(&self.data_tier.segments);
                linear.set_table(get_dm(), table)?;
                linear.resume(get_dm())?;
                false
            }
            _ => panic!("NOT (self.cache().is_some() AND self.linear.is_some())"),
        };

        if create {
            let table = map_to_dm(&self.data_tier.segments);
            let (dm_name, dm_uuid) = format_backstore_ids(pool_uuid, CacheRole::OriginSub);
            let origin = LinearDev::setup(get_dm(), &dm_name, Some(&dm_uuid), table)?;
            self.linear = Some(origin);
        }

        Ok(())
    }

    /// Satisfy a request for multiple segments. This request must
    /// always be satisfied exactly, None is returned if this can not
    /// be done.
    ///
    /// Precondition: self.next <= self.size()
    /// Postcondition: self.next <= self.size()
    ///
    /// Postcondition: forall i, sizes_i == result_i.1. The second value
    /// in each pair in the returned vector is therefore redundant, but is
    /// retained as a convenience to the caller.
    /// Postcondition:
    /// forall i, result_i.0 = result_(i - 1).0 + result_(i - 1).1
    ///
    /// WARNING: metadata changing event
    pub fn alloc(
        &mut self,
        pool_uuid: PoolUuid,
        sizes: &[Sectors],
    ) -> StratisResult<Option<Vec<(Sectors, Sectors)>>> {
        let total_required = sizes.iter().cloned().sum();
        let available = self.available_in_cap();
        if available < total_required {
            if self.data_tier.alloc(total_required - available) {
                self.extend_cap_device(pool_uuid)?;
            } else {
                return Ok(None);
            }
        }

        let mut chunks = Vec::new();
        for size in sizes {
            chunks.push((self.next, *size));
            self.next += *size;
        }

        // Assert that the postcondition holds.
        assert_eq!(
            sizes,
            chunks
                .iter()
                .map(|x| x.1)
                .collect::<Vec<Sectors>>()
                .as_slice()
        );

        Ok(Some(chunks))
    }

    /// Allocate a single segment from the backstore.
    /// If it is impossible to allocate the requested amount, try
    /// something smaller. If it is impossible to allocate any amount
    /// greater than 0, return None. Only allocate amounts that are divisible
    /// by modulus.
    ///
    /// Precondition: self.next <= self.size()
    /// Postcondition: self.next <= self.size()
    ///
    /// Postcondition: result.1 % modulus == 0
    /// Postcondition: result.1 <= request
    /// Postcondition: result.1 != 0
    ///
    /// WARNING: metadata changing event
    pub fn request(
        &mut self,
        pool_uuid: PoolUuid,
        request: Sectors,
        modulus: Sectors,
    ) -> StratisResult<Option<(Sectors, Sectors)>> {
        assert!(modulus != Sectors(0));

        let mut internal_request = (request / modulus) * modulus;

        if internal_request == Sectors(0) {
            return Ok(None);
        }

        let available = self.available_in_cap();
        if available < internal_request {
            let mut allocated = false;
            while !allocated && internal_request != Sectors(0) {
                allocated = self.data_tier.alloc(internal_request - available);
                let temp = internal_request / 2usize;
                internal_request = (temp / modulus) * modulus;
            }
            if allocated {
                self.extend_cap_device(pool_uuid)?;

                let return_amt = cmp::min(request, self.available_in_cap());
                let return_amt = (return_amt / modulus) * modulus;
                self.next += return_amt;
                Ok(Some((self.next - return_amt, return_amt)))
            } else {
                Ok(None)
            }
        } else {
            self.next += internal_request;
            Ok(Some((self.next - internal_request, internal_request)))
        }
    }

    /// Return a reference to all the blockdevs that this pool has ownership
    /// of. The blockdevs may be returned in any order. It is unsafe to assume
    /// that they are grouped by tier or any other organization.
    pub fn blockdevs(&self) -> Vec<(DevUuid, &StratBlockDev)> {
        match self.cache_tier {
            Some(ref cache) => cache
                .blockdevs()
                .iter()
                .chain(self.data_tier.blockdevs().iter())
                .cloned()
                .collect(),
            None => self.data_tier.blockdevs(),
        }
    }

    pub fn blockdevs_mut(&mut self) -> Vec<(DevUuid, &mut StratBlockDev)> {
        match self.cache_tier {
            Some(ref mut cache) => cache
                .blockdevs_mut()
                .into_iter()
                .chain(self.data_tier.blockdevs_mut().into_iter())
                .collect(),
            None => self.data_tier.blockdevs_mut(),
        }
    }

    /// The current size of all the blockdevs in the data tier.
    pub fn datatier_size(&self) -> Sectors {
        self.data_tier.size()
    }

    /// The size of the cap device.
    ///
    /// The size of the cap device is obtained from the size of the component
    /// DM devices. But the devicemapper library stores the data from which
    /// the size of each DM device is calculated; the result is computed and
    /// no ioctl is required.
    fn size(&self) -> Sectors {
        self.linear
            .as_ref()
            .map(|d| d.size())
            .or_else(|| self.cache.as_ref().map(|d| d.size()))
            .unwrap_or(Sectors(0))
    }

    /// The total number of unallocated usable sectors in the
    /// backstore. Includes both in the cap but unallocated as well as not yet
    /// added to cap.
    pub fn available_in_backstore(&self) -> Sectors {
        self.data_tier.usable_size() - self.next
    }

    /// The available number of Sectors.
    fn available_in_cap(&self) -> Sectors {
        let size = self.size();
        // It is absolutely essential for correct operation that the assertion
        // be true. If it is false, the result will be incorrect, and space
        // will be allocated incorrectly from the cap device.
        assert!(
            self.next <= size,
            format!(
                "next index, {}, is greater than the total size available {}",
                self.next, size
            )
        );
        size - self.next
    }

    /// Destroy the entire store.
    pub fn destroy(&mut self) -> StratisResult<()> {
        match self.cache {
            Some(ref mut cache) => {
                cache.teardown(get_dm())?;
                self.cache_tier
                    .as_mut()
                    .expect("if dm_device is cache, cache tier exists")
                    .destroy()?;
            }
            None => {
                if let Some(ref mut linear) = self.linear {
                    linear.teardown(get_dm())?;
                }
            }
        };
        self.data_tier.destroy()
    }

    /// Teardown the DM devices in the backstore.
    #[cfg(test)]
    pub fn teardown(&mut self) -> StratisResult<()> {
        match self.cache {
            Some(ref mut cache) => cache.teardown(get_dm()),
            None => if let Some(ref mut linear) = self.linear {
                linear.teardown(get_dm())
            } else {
                Ok(())
            },
        }.map_err(|e| e.into())
    }

    /// Return the device that this tier is currently using.
    /// This changes, depending on whether the backstore is supporting a cache
    /// or not. There may be no device if no data has yet been allocated from
    /// the backstore.
    pub fn device(&self) -> Option<Device> {
        self.cache
            .as_ref()
            .map(|d| d.device())
            .or_else(|| self.linear.as_ref().map(|d| d.device()))
    }

    /// Lookup an immutable blockdev by its Stratis UUID.
    pub fn get_blockdev_by_uuid(&self, uuid: DevUuid) -> Option<(BlockDevTier, &StratBlockDev)> {
        self.data_tier.get_blockdev_by_uuid(uuid).or_else(|| {
            self.cache_tier
                .as_ref()
                .and_then(|c| c.get_blockdev_by_uuid(uuid))
        })
    }

    /// Lookup a mutable blockdev by its Stratis UUID.
    pub fn get_mut_blockdev_by_uuid(
        &mut self,
        uuid: DevUuid,
    ) -> Option<(BlockDevTier, &mut StratBlockDev)> {
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
    pub fn datatier_metadata_size(&self) -> Sectors {
        self.data_tier.metadata_size()
    }

    /// Write the given data to the data tier's devices.
    pub fn save_state(&mut self, metadata: &[u8]) -> StratisResult<()> {
        self.data_tier.save_state(metadata)
    }

    /// Set user info field on the specified blockdev.
    /// May return an error if there is no blockdev for the given UUID.
    pub fn set_blockdev_user_info(
        &mut self,
        uuid: DevUuid,
        user_info: Option<&str>,
    ) -> StratisResult<bool> {
        self.get_mut_blockdev_by_uuid(uuid).map_or_else(
            || {
                Err(StratisError::Engine(
                    ErrorEnum::NotFound,
                    format!("No blockdev for uuid {} found", uuid),
                ))
            },
            |(_, b)| Ok(b.set_user_info(user_info)),
        )
    }
}

impl Recordable<BackstoreSave> for Backstore {
    fn record(&self) -> BackstoreSave {
        BackstoreSave {
            cache_tier: self.cache_tier.as_ref().map(|c| c.record()),
            cap: CapSave {
                allocs: vec![(Sectors(0), self.next)],
            },
            data_tier: self.data_tier.record(),
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use devicemapper::{CacheDevStatus, DataBlocks, IEC};

    use super::super::blockdevmgr::Segment;

    use super::super::super::cmd;
    use super::super::super::tests::{loopbacked, real};

    use super::super::setup::find_all;

    use super::*;

    const INITIAL_BACKSTORE_ALLOCATION: Sectors = CACHE_BLOCK_SIZE;

    /// Assert some invariants of the backstore
    /// * backstore.cache_tier.is_some() <=> backstore.cache.is_some() &&
    ///   backstore.cache_tier.is_some() => backstore.linear.is_none()
    /// * backstore's data tier allocated is equal to the size of the cap device
    /// * backstore's next index is always less than the size of the cap
    ///   device
    fn invariant(backstore: &Backstore) -> () {
        assert!(
            (backstore.cache_tier.is_none() && backstore.cache.is_none())
                || (backstore.cache_tier.is_some()
                    && backstore.cache.is_some()
                    && backstore.linear.is_none())
        );
        assert_eq!(
            backstore.data_tier.allocated(),
            match (&backstore.linear, &backstore.cache) {
                (None, None) => Sectors(0),
                (&None, &Some(ref cache)) => cache.size(),
                (&Some(ref linear), &None) => linear.size(),
                _ => panic!("impossible; see first assertion"),
            }
        );
        assert!(backstore.next <= backstore.size())
    }

    /// Test adding cachedevs to the backstore.
    /// When cachedevs are added, cache tier, etc. must exist.
    /// Nonetheless, because nothing is written or read, cache usage ought
    /// to be 0. Adding some more cachedevs exercises different code path
    /// from adding initial cachedevs.
    fn test_add_cache_devs(paths: &[&Path]) -> () {
        assert!(paths.len() > 3);

        let meta_size = Sectors(IEC::Mi);

        let (initcachepaths, paths) = paths.split_at(1);
        let (cachedevpaths, paths) = paths.split_at(1);
        let (datadevpaths, initdatapaths) = paths.split_at(1);

        let pool_uuid = Uuid::new_v4();
        let mut backstore =
            Backstore::initialize(pool_uuid, initdatapaths, MIN_MDA_SECTORS).unwrap();

        invariant(&backstore);

        // Allocate space from the backstore so that the cap device is made.
        backstore
            .alloc(pool_uuid, &[INITIAL_BACKSTORE_ALLOCATION])
            .unwrap();

        let cache_uuids = backstore.add_cachedevs(pool_uuid, initcachepaths).unwrap();

        invariant(&backstore);

        assert_eq!(cache_uuids.len(), initcachepaths.len());
        assert!(backstore.linear.is_none());

        let cache_status = backstore
            .cache
            .as_ref()
            .map(|c| c.status(get_dm()).unwrap())
            .unwrap();

        match cache_status {
            CacheDevStatus::Working(status) => {
                let usage = &status.usage;
                assert_eq!(usage.used_cache, DataBlocks(0));
                assert_eq!(usage.total_meta, meta_size.metablocks());
                assert!(usage.total_cache > DataBlocks(0));
            }
            CacheDevStatus::Fail => panic!("cache status should succeed"),
        }

        let data_uuids = backstore.add_datadevs(pool_uuid, datadevpaths).unwrap();
        invariant(&backstore);
        assert_eq!(data_uuids.len(), datadevpaths.len());

        let cache_uuids = backstore.add_cachedevs(pool_uuid, cachedevpaths).unwrap();
        invariant(&backstore);
        assert_eq!(cache_uuids.len(), cachedevpaths.len());

        let cache_status = backstore
            .cache
            .as_ref()
            .map(|c| c.status(get_dm()).unwrap())
            .unwrap();

        match cache_status {
            CacheDevStatus::Working(status) => {
                let usage = &status.usage;
                assert_eq!(usage.used_cache, DataBlocks(0));
                assert_eq!(usage.total_meta, meta_size.metablocks());
                assert!(usage.total_cache > DataBlocks(0));
            }
            CacheDevStatus::Fail => panic!("cache status should succeed"),
        }

        backstore.destroy().unwrap();
    }

    #[test]
    pub fn loop_test_add_cache_devs() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(4, 5, None),
            test_add_cache_devs,
        );
    }

    #[test]
    pub fn real_test_add_cache_devs() {
        real::test_with_spec(
            real::DeviceLimits::AtLeast(4, None, None),
            test_add_cache_devs,
        );
    }

    #[test]
    pub fn travis_test_add_cache_devs() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(4, 5, None),
            test_add_cache_devs,
        );
    }

    /// Create a backstore.
    /// Request a amount that can not be allocated because the modulus is
    /// bigger than the reqested amount.
    /// Request an impossibly large amount.
    /// Verify that the backstore is now all used up.
    fn test_request(paths: &[&Path]) -> () {
        assert!(paths.len() > 0);

        let pool_uuid = Uuid::new_v4();
        let mut backstore = Backstore::initialize(pool_uuid, paths, MIN_MDA_SECTORS).unwrap();

        assert!(
            backstore
                .request(pool_uuid, Sectors(IEC::Ki), Sectors(IEC::Mi))
                .unwrap()
                .is_none()
        );

        let request = Sectors(IEC::Ei);
        let modulus = Sectors(IEC::Ki);
        let old_next = backstore.next;
        let (start, length) = backstore
            .request(pool_uuid, request, modulus)
            .unwrap()
            .unwrap();
        assert!(length < request);
        assert_eq!(length % modulus, Sectors(0));
        assert_eq!(backstore.next, old_next + length);
        assert_eq!(start, old_next);

        let new_request = backstore
            .request(pool_uuid, request, Sectors(IEC::Ki))
            .unwrap();

        // Either there is nothing left to allocate or there is some, but it
        // is less than length.  If what is allocated now is more than length
        // then the amount available to be allocated was greater than
        // length * 2. In that case, length * 2 would have been allocated.
        assert!(new_request.is_none() || new_request.expect("!is_none()").1 < length);
        cmd::udev_settle().unwrap();
        backstore.destroy().unwrap();
        cmd::udev_settle().unwrap();
    }

    #[test]
    pub fn loop_test_request() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(1, 3, None), test_request);
    }

    #[test]
    pub fn real_test_request() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1, None, None), test_request);
    }

    #[test]
    pub fn travis_test_request() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(1, 3, None), test_request);
    }

    /// Create a backstore with a cache.
    /// Setup the same backstore, should succeed.
    /// Verify that blockdev metadatas are the same for the backstores.
    /// Tear down the backstore.
    /// Setup the same backstore again.
    /// Verify blockdev metadata again.
    /// Destroy all.
    fn test_setup(paths: &[&Path]) -> () {
        assert!(paths.len() > 1);

        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let pool_uuid = Uuid::new_v4();

        let mut backstore = Backstore::initialize(pool_uuid, paths1, MIN_MDA_SECTORS).unwrap();
        invariant(&backstore);

        // Allocate space from the backstore so that the cap device is made.
        backstore
            .alloc(pool_uuid, &[INITIAL_BACKSTORE_ALLOCATION])
            .unwrap();

        let old_device = backstore.device();

        backstore.add_cachedevs(pool_uuid, paths2).unwrap();
        invariant(&backstore);

        assert_ne!(backstore.device(), old_device);

        let backstore_save = backstore.record();

        cmd::udev_settle().unwrap();
        let map = find_all().unwrap();
        let map = map.get(&pool_uuid).unwrap();
        let mut backstore = Backstore::setup(pool_uuid, &backstore_save, &map, None).unwrap();
        invariant(&backstore);

        let backstore_save2 = backstore.record();
        assert_eq!(backstore_save.cache_tier, backstore_save2.cache_tier);
        assert_eq!(backstore_save.data_tier, backstore_save2.data_tier);

        backstore.teardown().unwrap();

        cmd::udev_settle().unwrap();
        let map = find_all().unwrap();
        let map = map.get(&pool_uuid).unwrap();
        let mut backstore = Backstore::setup(pool_uuid, &backstore_save, &map, None).unwrap();
        invariant(&backstore);

        let backstore_save2 = backstore.record();
        assert_eq!(backstore_save.cache_tier, backstore_save2.cache_tier);
        assert_eq!(backstore_save.data_tier, backstore_save2.data_tier);

        backstore.destroy().unwrap();
    }

    #[test]
    pub fn loop_test_setup() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(2, 3, None), test_setup);
    }

    #[test]
    pub fn real_test_setup() {
        real::test_with_spec(real::DeviceLimits::AtLeast(2, None, None), test_setup);
    }

    #[test]
    pub fn travis_test_setup() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(2, 3, None), test_setup);
    }

    #[test]
    fn test_constrain_max_cache_size() {
        // segment with one sector
        let small_seg = BlkDevSegment {
            uuid: Uuid::new_v4(),
            segment: Segment::new(Device::from(1), Sectors(0), Sectors(1)),
        };
        // segment with MAX_CACHE_SIZE - 1 sector
        let large_seg = BlkDevSegment {
            uuid: Uuid::new_v4(),
            segment: Segment::new(Device::from(1), Sectors(0), MAX_CACHE_SIZE - Sectors(1)),
        };
        // largest possible segment
        let max_seg = BlkDevSegment {
            uuid: Uuid::new_v4(),
            segment: Segment::new(Device::from(1), Sectors(0), Sectors(u64::max_value())),
        };

        assert_eq!(constrain_max_cache_size(&[large_seg.clone()]).len(), 1);
        assert_eq!(
            constrain_max_cache_size(&[large_seg.clone(), large_seg.clone()]).len(),
            1
        );
        assert_eq!(
            constrain_max_cache_size(&[large_seg.clone(), small_seg.clone()]).len(),
            2
        );
        assert_eq!(
            constrain_max_cache_size(&[large_seg.clone(), small_seg.clone(), small_seg.clone()])
                .len(),
            2
        );
        assert_eq!(
            constrain_max_cache_size(&[small_seg.clone(), large_seg.clone()]).len(),
            2
        );
        assert_eq!(
            constrain_max_cache_size(&[small_seg.clone(), small_seg.clone(), large_seg.clone()])
                .len(),
            2
        );
        assert_eq!(
            constrain_max_cache_size(&[small_seg.clone(), small_seg.clone(), small_seg.clone()])
                .len(),
            3
        );
        assert_eq!(
            constrain_max_cache_size(&[small_seg.clone(), small_seg.clone(), max_seg.clone()])
                .len(),
            2
        );
    }
}

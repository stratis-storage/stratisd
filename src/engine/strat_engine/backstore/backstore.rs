// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle the backing store of a pool.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use devicemapper::{CacheDev, Device, DmDevice, LinearDev, Sectors};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::types::{BlockDevTier, DevUuid, PoolUuid};

use super::super::dm::get_dm;
use super::super::serde_structs::{BackstoreSave, Recordable};

use super::blockdev::StratBlockDev;
use super::blockdevmgr::BlockDevMgr;
use super::cache_tier::CacheTier;
use super::data_tier::DataTier;
use super::metadata::MIN_MDA_SECTORS;
use super::setup::get_blockdevs;

/// This structure can allocate additional space to the upper layer, but it
/// cannot accept returned space. When it is extended to be able to accept
/// returned space the allocation algorithm will have to be revised.
///
/// self.linear.is_some() XOR self.cache.is_some()
/// self.cache.is_some() <=> self.cache_tier.is_some()
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

impl Backstore {
    /// Make a Backstore object from blockdevs that already belong to Stratis.
    /// Precondition: every device in devnodes has already been determined to
    /// belong to the pool with the specified pool_uuid.
    /// Precondition: next <= the sum of the lengths of the segments allocated
    /// to the data tier cap device.
    pub fn setup(
        pool_uuid: PoolUuid,
        backstore_save: &BackstoreSave,
        devnodes: &HashMap<Device, PathBuf>,
        last_update_time: Option<DateTime<Utc>>,
        next: Sectors,
    ) -> StratisResult<Backstore> {
        let (datadevs, cachedevs) = get_blockdevs(pool_uuid, backstore_save, devnodes)?;
        let block_mgr = BlockDevMgr::new(datadevs, last_update_time);
        let (data_tier, dm_device) =
            DataTier::setup(pool_uuid, block_mgr, &backstore_save.data_segments)?;

        let (cache_tier, cache, linear) = if !cachedevs.is_empty() {
            let block_mgr = BlockDevMgr::new(cachedevs, last_update_time);
            match (
                &backstore_save.cache_segments,
                &backstore_save.meta_segments,
            ) {
                (&Some(ref cache_segments), &Some(ref meta_segments)) => {
                    let (cache_tier, cache_device) = CacheTier::setup(
                        pool_uuid,
                        block_mgr,
                        dm_device,
                        cache_segments,
                        meta_segments,
                    )?;
                    (Some(cache_tier), Some(cache_device), None)
                }
                _ => {
                    let err_msg = "Cachedevs exist, but meta or cache segments are not allocated";
                    return Err(StratisError::Engine(ErrorEnum::Error, err_msg.into()));
                }
            }
        } else {
            (None, None, Some(dm_device))
        };

        Ok(Backstore {
            data_tier,
            cache_tier,
            linear,
            cache,
            next,
        })
    }

    /// Initialize a Backstore object, by initializing the specified devs.
    /// WARNING: metadata changing event
    pub fn initialize(
        pool_uuid: PoolUuid,
        paths: &[&Path],
        mda_size: Sectors,
        force: bool,
    ) -> StratisResult<Backstore> {
        let (data_tier, dm_device) = DataTier::new(
            pool_uuid,
            BlockDevMgr::initialize(pool_uuid, paths, mda_size, force)?,
        )?;
        Ok(Backstore {
            data_tier,
            cache_tier: None,
            linear: Some(dm_device),
            cache: None,
            next: Sectors(0),
        })
    }

    /// Add cachedevs to the backstore.
    ///
    /// If the cache tier does not already exist, create it.
    ///
    // Postcondition: self.cache.is_some() && self.linear.is_none()
    fn add_cachedevs(
        &mut self,
        pool_uuid: PoolUuid,
        paths: &[&Path],
        force: bool,
    ) -> StratisResult<Vec<DevUuid>> {
        match self.cache_tier {
            Some(ref mut cache_tier) => {
                let mut cache_device = self.cache
                    .as_mut()
                    .expect("cache_tier.is_some() <=> self.cache.is_some()");
                cache_tier.add(pool_uuid, &mut cache_device, paths, force)
            }
            None => {
                let bdm = BlockDevMgr::initialize(pool_uuid, paths, MIN_MDA_SECTORS, force)?;

                let linear = self.linear
                    .take()
                    .expect("cache_tier.is_none() <=> self.linear.is_some()");
                let (cache_tier, cache) = CacheTier::new(pool_uuid, bdm, linear)?;
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
    fn add_datadevs(
        &mut self,
        pool_uuid: PoolUuid,
        paths: &[&Path],
        force: bool,
    ) -> StratisResult<Vec<DevUuid>> {
        self.data_tier.add(
            pool_uuid,
            self.cache.as_mut(),
            self.linear.as_mut(),
            paths,
            force,
        )
    }

    /// Add the given paths to self. Return UUIDs of the new blockdevs
    /// corresponding to the specified paths.
    /// WARNING: metadata changing event
    pub fn add_blockdevs(
        &mut self,
        pool_uuid: PoolUuid,
        paths: &[&Path],
        tier: BlockDevTier,
        force: bool,
    ) -> StratisResult<Vec<DevUuid>> {
        match tier {
            BlockDevTier::Cache => self.add_cachedevs(pool_uuid, paths, force),
            BlockDevTier::Data => self.add_datadevs(pool_uuid, paths, force),
        }
    }

    /// Allocate requested chunks from device.
    /// Returns None if it is not possible to satisfy the request.
    /// Each segment allocated is contiguous with its neighbors in the return
    /// vector.
    /// Precondition: self.next <= self.size()
    /// Postcondition: self.next <= self.size()
    /// WARNING: All this must change when it becomes possible to return
    /// sectors to the store.
    /// WARNING: metadata changing event
    pub fn alloc_space(&mut self, sizes: &[Sectors]) -> Option<Vec<(Sectors, Sectors)>> {
        if self.available() < sizes.iter().cloned().sum() {
            return None;
        }

        let mut chunks = Vec::new();
        for size in sizes {
            chunks.push((self.next, *size));
            self.next += *size;
        }
        Some(chunks)
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

    /// The current capacity of all the blockdevs in the data tier.
    pub fn datatier_current_capacity(&self) -> Sectors {
        self.data_tier.current_capacity()
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
            .expect("either linear or cache must be Some")
    }

    /// The available number of Sectors.
    pub fn available(&self) -> Sectors {
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
    pub fn destroy(self) -> StratisResult<()> {
        match self.cache {
            Some(cache) => {
                cache.teardown(get_dm())?;
                self.cache_tier
                    .expect("if dm_device is cache, cache tier exists")
                    .destroy()?;
            }
            None => {
                self.linear
                    .expect("self.cache.is_none()")
                    .teardown(get_dm())?;
            }
        };
        self.data_tier.destroy()
    }

    /// Teardown the DM devices in the backstore.
    #[cfg(test)]
    pub fn teardown(self) -> StratisResult<()> {
        match self.cache {
            Some(cache) => cache.teardown(get_dm()),
            None => self.linear
                .expect("self.cache.is_none()")
                .teardown(get_dm()),
        }.map_err(|e| e.into())
    }

    /// Return the device that this tier is currently using.
    /// This changes, depending on whether the backstore is supporting a cache
    /// or not.
    pub fn device(&self) -> Device {
        self.cache
            .as_ref()
            .map(|d| d.device())
            .or_else(|| self.linear.as_ref().map(|d| d.device()))
            .expect("must be one or the other")
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
    fn get_mut_blockdev_by_uuid(
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
            cache_devs: self.cache_tier.as_ref().map(|c| c.block_mgr.record()),
            cache_segments: self.cache_tier.as_ref().map(|c| c.cache_segments.record()),
            data_devs: self.data_tier.block_mgr.record(),
            data_segments: self.data_tier.segments.record(),
            meta_segments: self.cache_tier.as_ref().map(|c| c.meta_segments.record()),
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use devicemapper::{CacheDevStatus, DataBlocks, IEC};

    use super::super::super::cmd;
    use super::super::super::tests::{loopbacked, real};

    use super::super::setup::find_all;

    use super::*;

    /// Assert some invariants of the backstore
    /// * self.cache_tier.is_some() && self.cache.is_some() XOR
    ///   self.linear.is_some()).
    /// * self.data_tier.block_mgr.avail_space() is always 0, because
    ///   everything is allocated to the DM device.
    /// * backstore's data tier capacity is equal to the size of the cap device
    /// * backstore's next index is always less than the size of the cap
    ///   device
    fn invariant(backstore: &Backstore) -> () {
        assert!(
            (backstore.cache_tier.is_none()
                && backstore.cache.is_none()
                && backstore.linear.is_some())
                || (backstore.cache_tier.is_some()
                    && backstore.cache.is_some()
                    && backstore.linear.is_none())
        );
        assert_eq!(backstore.data_tier.block_mgr.avail_space(), Sectors(0));
        assert_eq!(
            backstore.data_tier.capacity(),
            match (&backstore.linear, &backstore.cache) {
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
            Backstore::initialize(pool_uuid, initdatapaths, MIN_MDA_SECTORS, false).unwrap();

        invariant(&backstore);

        let cache_uuids = backstore
            .add_blockdevs(pool_uuid, initcachepaths, BlockDevTier::Cache, false)
            .unwrap();

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

        let data_uuids = backstore
            .add_blockdevs(pool_uuid, datadevpaths, BlockDevTier::Data, false)
            .unwrap();
        invariant(&backstore);
        assert_eq!(data_uuids.len(), datadevpaths.len());

        let cache_uuids = backstore
            .add_blockdevs(pool_uuid, cachedevpaths, BlockDevTier::Cache, false)
            .unwrap();
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

        let mut backstore =
            Backstore::initialize(pool_uuid, paths1, MIN_MDA_SECTORS, false).unwrap();
        invariant(&backstore);
        let old_device = backstore.device();

        backstore
            .add_blockdevs(pool_uuid, paths2, BlockDevTier::Cache, false)
            .unwrap();
        invariant(&backstore);

        assert!(backstore.device() != old_device);

        let backstore_save = backstore.record();

        cmd::udev_settle().unwrap();
        let map = find_all().unwrap();
        let map = map.get(&pool_uuid).unwrap();
        let backstore =
            Backstore::setup(pool_uuid, &backstore_save, &map, None, Sectors(0)).unwrap();
        invariant(&backstore);

        let backstore_save2 = backstore.record();
        assert_eq!(backstore_save.cache_devs, backstore_save2.cache_devs);
        assert_eq!(backstore_save.data_devs, backstore_save2.data_devs);

        backstore.teardown().unwrap();

        cmd::udev_settle().unwrap();
        let map = find_all().unwrap();
        let map = map.get(&pool_uuid).unwrap();
        let backstore =
            Backstore::setup(pool_uuid, &backstore_save, &map, None, Sectors(0)).unwrap();
        invariant(&backstore);

        let backstore_save2 = backstore.record();
        assert_eq!(backstore_save.cache_devs, backstore_save2.cache_devs);
        assert_eq!(backstore_save.data_devs, backstore_save2.data_devs);

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
}

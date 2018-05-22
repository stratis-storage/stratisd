// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle the backing store of a pool.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use devicemapper::{CacheDev, Device, DmDevice, LinearDev, Sectors, IEC};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::types::{BlockDevTier, DevUuid, PoolUuid};

use super::super::device::wipe_sectors;
use super::super::dm::get_dm;
use super::super::dmnames::{format_backstore_ids, CacheRole};
use super::super::serde_structs::{BackstoreSave, Recordable};

use super::blockdev::StratBlockDev;
use super::blockdevmgr::{coalesce_blkdevsegs, map_to_dm, BlkDevSegment, BlockDevMgr, Segment};
use super::metadata::MIN_MDA_SECTORS;
use super::setup::get_blockdevs;

/// Use a cache block size that the kernel docs indicate is the largest
/// typical size.
const CACHE_BLOCK_SIZE: Sectors = Sectors(2048); // 1024 KiB

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
    pub fn setup(
        block_mgr: BlockDevMgr,
        segments: &[(DevUuid, Sectors, Sectors)],
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

        let (dm_name, dm_uuid) = format_backstore_ids(block_mgr.pool_uuid(), CacheRole::OriginSub);
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
    ///
    /// WARNING: metadata changing event
    pub fn new(mut block_mgr: BlockDevMgr) -> StratisResult<(DataTier, LinearDev)> {
        let avail_space = block_mgr.avail_space();
        let segments = block_mgr
            .alloc_space(&[avail_space])
            .expect("asked for exactly the space available, must get")
            .iter()
            .flat_map(|s| s.iter())
            .cloned()
            .collect::<Vec<_>>();
        let (dm_name, dm_uuid) = format_backstore_ids(block_mgr.pool_uuid(), CacheRole::OriginSub);
        let ld = LinearDev::setup(get_dm(), &dm_name, Some(&dm_uuid), map_to_dm(&segments))?;
        Ok((
            DataTier {
                block_mgr,
                segments,
            },
            ld,
        ))
    }

    /// Add the given paths to self. Return UUIDs of the new blockdevs
    /// corresponding to the specified paths.
    /// WARNING: metadata changing event
    pub fn add(
        &mut self,
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
}

/// Handles the cache devices.
#[derive(Debug)]
struct CacheTier {
    /// Manages the individual block devices
    block_mgr: BlockDevMgr,
    /// The list of segments granted by block_mgr and used by the cache
    /// device.
    cache_segments: Vec<BlkDevSegment>,
    /// The list of segments granted by block_mgr and used by the metadata
    /// device.
    meta_segments: Vec<BlkDevSegment>,
}

impl CacheTier {
    /// Setup a previously existing cache layer from the block_mgr and
    /// previously allocated segments.
    ///
    /// Returns the CacheTier and the cache DM device that was created during
    /// setup.
    pub fn setup(
        block_mgr: BlockDevMgr,
        origin: LinearDev,
        cache_segments: &[(DevUuid, Sectors, Sectors)],
        meta_segments: &[(DevUuid, Sectors, Sectors)],
    ) -> StratisResult<(CacheTier, CacheDev)> {
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

        let meta_segments = meta_segments
            .iter()
            .map(&mapper)
            .collect::<StratisResult<Vec<_>>>()?;
        let (dm_name, dm_uuid) = format_backstore_ids(block_mgr.pool_uuid(), CacheRole::MetaSub);
        let meta = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            map_to_dm(&meta_segments),
        )?;

        let cache_segments = cache_segments
            .iter()
            .map(&mapper)
            .collect::<StratisResult<Vec<_>>>()?;
        let (dm_name, dm_uuid) = format_backstore_ids(block_mgr.pool_uuid(), CacheRole::CacheSub);
        let cache = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            map_to_dm(&cache_segments),
        )?;

        let (dm_name, dm_uuid) = format_backstore_ids(block_mgr.pool_uuid(), CacheRole::Cache);
        let cd = CacheDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            meta,
            cache,
            origin,
            CACHE_BLOCK_SIZE,
        )?;

        Ok((
            CacheTier {
                block_mgr,
                meta_segments,
                cache_segments,
            },
            cd,
        ))
    }

    /// Add the given paths to self. Return UUIDs of the new blockdevs
    /// corresponding to the specified paths.
    /// Adds all additional space to cache sub-device.
    /// WARNING: metadata changing event
    // FIXME: That all segments on the newly added device are added to the
    // cache sub-device and none to the meta sub-device could lead to failure.
    // Presumably, the size required for the meta sub-device varies directly
    // with the size of cache sub-device.
    pub fn add(
        &mut self,
        cache_device: &mut CacheDev,
        paths: &[&Path],
        force: bool,
    ) -> StratisResult<Vec<DevUuid>> {
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
        let table = map_to_dm(&coalesced);

        cache_device.set_cache_table(get_dm(), table)?;
        cache_device.resume(get_dm())?;

        self.cache_segments = coalesced;

        Ok(uuids)
    }

    /// Setup a new CacheTier struct from the block_mgr.
    ///
    /// Returns the CacheTier and the cache device that was created.
    ///
    /// WARNING: metadata changing event
    pub fn new(
        mut block_mgr: BlockDevMgr,
        origin: LinearDev,
    ) -> StratisResult<(CacheTier, CacheDev)> {
        let avail_space = block_mgr.avail_space();

        // FIXME: Come up with a better way to choose metadata device size
        let meta_space = Sectors(IEC::Mi);

        assert!(
            meta_space < avail_space,
            "every block device must be at least one GiB"
        );

        let mut segments = block_mgr
            .alloc_space(&[meta_space, avail_space - meta_space])
            .expect("asked for exactly the space available, must get");

        let cache_segments = segments.pop().expect("segments.len() == 2");
        let meta_segments = segments.pop().expect("segments.len() == 1");

        let (dm_name, dm_uuid) = format_backstore_ids(block_mgr.pool_uuid(), CacheRole::MetaSub);
        let meta = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            map_to_dm(&meta_segments),
        )?;

        // See comment in ThinPool::new() method
        wipe_sectors(&meta.devnode(), Sectors(0), meta.size())?;

        let (dm_name, dm_uuid) = format_backstore_ids(block_mgr.pool_uuid(), CacheRole::CacheSub);
        let cache = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            map_to_dm(&cache_segments),
        )?;

        let (dm_name, dm_uuid) = format_backstore_ids(block_mgr.pool_uuid(), CacheRole::Cache);
        let cd = CacheDev::new(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            meta,
            cache,
            origin,
            CACHE_BLOCK_SIZE,
        )?;

        Ok((
            CacheTier {
                block_mgr,
                meta_segments,
                cache_segments,
            },
            cd,
        ))
    }

    /// Destroy the tier. Wipe its blockdevs.
    pub fn destroy(self) -> StratisResult<()> {
        self.block_mgr.destroy_all()
    }

    /// Get all the blockdevs belonging to this tier.
    pub fn blockdevs(&self) -> Vec<(DevUuid, &StratBlockDev)> {
        self.block_mgr.blockdevs()
    }

    /// Lookup an immutable blockdev by its Stratis UUID.
    pub fn get_blockdev_by_uuid(&self, uuid: DevUuid) -> Option<(BlockDevTier, &StratBlockDev)> {
        self.block_mgr
            .get_blockdev_by_uuid(uuid)
            .and_then(|bd| Some((BlockDevTier::Cache, bd)))
    }

    /// Lookup a mutable blockdev by its Stratis UUID.
    pub fn get_mut_blockdev_by_uuid(
        &mut self,
        uuid: DevUuid,
    ) -> Option<(BlockDevTier, &mut StratBlockDev)> {
        self.block_mgr
            .get_mut_blockdev_by_uuid(uuid)
            .and_then(|bd| Some((BlockDevTier::Cache, bd)))
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
    /// A cache DM Device.
    cache: Option<CacheDev>,
    /// Coordinate handling of blockdevs that back the cache. Optional, since
    /// this structure can operate without a cache.
    cache_tier: Option<CacheTier>,
    /// Coordinates handling of the blockdevs that form the base.
    data_tier: DataTier,
    /// A linear DM device.
    linear: Option<LinearDev>,
    /// Index for managing allocation from dm_device.
    next: Sectors,
}

impl Backstore {
    /// Make a Backstore object from blockdevs that already belong to Stratis.
    pub fn setup(
        pool_uuid: PoolUuid,
        backstore_save: &BackstoreSave,
        devnodes: &HashMap<Device, PathBuf>,
        last_update_time: Option<DateTime<Utc>>,
    ) -> StratisResult<Backstore> {
        let (datadevs, cachedevs) = get_blockdevs(pool_uuid, backstore_save, devnodes)?;
        let block_mgr = BlockDevMgr::new(pool_uuid, datadevs, last_update_time);
        let (data_tier, dm_device) = DataTier::setup(block_mgr, &backstore_save.data_segments)?;

        let (cache_tier, cache, linear) = if !cachedevs.is_empty() {
            let block_mgr = BlockDevMgr::new(pool_uuid, cachedevs, last_update_time);
            match (
                &backstore_save.cache_segments,
                &backstore_save.meta_segments,
            ) {
                (&Some(ref cache_segments), &Some(ref meta_segments)) => {
                    let (cache_tier, cache_device) =
                        CacheTier::setup(block_mgr, dm_device, cache_segments, meta_segments)?;
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
            next: backstore_save.next,
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
        let (data_tier, dm_device) =
            DataTier::new(BlockDevMgr::initialize(pool_uuid, paths, mda_size, force)?)?;
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
    fn add_cachedevs(&mut self, paths: &[&Path], force: bool) -> StratisResult<Vec<DevUuid>> {
        match self.cache_tier {
            Some(ref mut cache_tier) => {
                let mut cache_device = self.cache
                    .as_mut()
                    .expect("cache_tier.is_some() <=> self.cache.is_some()");
                cache_tier.add(&mut cache_device, paths, force)
            }
            None => {
                // FIXME: This is obviously a bad idea, but unless the UUID
                // is obtained from block_mgr then it is necessary to
                // add a pool UUID parameter to this method. That course of
                // action would require various changes in the calling code
                // which have not been agreed upon. See GitHub PR:
                // https://github.com/stratis-storage/stratisd/pull/784
                let pool_uuid = self.data_tier.block_mgr.pool_uuid();
                let bdm = BlockDevMgr::initialize(pool_uuid, paths, MIN_MDA_SECTORS, force)?;

                let linear = self.linear
                    .take()
                    .expect("cache_tier.is_none() <=> self.linear.is_some()");
                let (cache_tier, cache) = CacheTier::new(bdm, linear)?;
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

    /// Add the given paths to self. Return UUIDs of the new blockdevs
    /// corresponding to the specified paths.
    /// WARNING: metadata changing event
    pub fn add_blockdevs(
        &mut self,
        paths: &[&Path],
        tier: BlockDevTier,
        force: bool,
    ) -> StratisResult<Vec<DevUuid>> {
        match tier {
            BlockDevTier::Cache => self.add_cachedevs(paths, force),
            BlockDevTier::Data => {
                self.data_tier
                    .add(self.cache.as_mut(), self.linear.as_mut(), paths, force)
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

    /// The available number of Sectors.
    pub fn available(&self) -> Sectors {
        self.data_tier.capacity() - self.next
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
            next: self.next,
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use devicemapper::{CacheDevStatus, DataBlocks};

    use super::super::super::tests::{loopbacked, real};

    use super::super::setup::find_all;

    use super::*;

    /// Assert some invariants of the backstore
    /// * self.cache_tier.is_some() && self.cache.is_some() XOR
    ///   self.linear.is_some()).
    /// * self.data_tier.block_mgr.avail_space() is always 0, because
    ///   everything is allocated to the DM device.
    fn invariant(backstore: &Backstore) -> () {
        assert!(
            (backstore.cache_tier.is_none() && backstore.cache.is_none()
                && backstore.linear.is_some())
                || (backstore.cache_tier.is_some() && backstore.cache.is_some()
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
        )
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

        let mut backstore =
            Backstore::initialize(Uuid::new_v4(), initdatapaths, MIN_MDA_SECTORS, false).unwrap();

        invariant(&backstore);

        let cache_uuids = backstore
            .add_blockdevs(initcachepaths, BlockDevTier::Cache, false)
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
            .add_blockdevs(datadevpaths, BlockDevTier::Data, false)
            .unwrap();
        invariant(&backstore);
        assert_eq!(data_uuids.len(), datadevpaths.len());

        let cache_uuids = backstore
            .add_blockdevs(cachedevpaths, BlockDevTier::Cache, false)
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
    /// Tear down the backstore.
    /// Setup the same backstore again.
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
            .add_blockdevs(paths2, BlockDevTier::Cache, false)
            .unwrap();
        invariant(&backstore);

        assert!(backstore.device() != old_device);

        let backstore_save = backstore.record();

        let map = find_all().unwrap();
        let map = map.get(&pool_uuid).unwrap();
        let backstore = Backstore::setup(pool_uuid, &backstore_save, &map, None).unwrap();
        invariant(&backstore);

        backstore.teardown().unwrap();

        let map = find_all().unwrap();
        let map = map.get(&pool_uuid).unwrap();
        let backstore = Backstore::setup(pool_uuid, &backstore_save, &map, None).unwrap();
        invariant(&backstore);

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

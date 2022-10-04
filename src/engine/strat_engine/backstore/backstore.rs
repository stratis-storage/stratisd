// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle the backing store of a pool.

use std::cmp;

use chrono::{DateTime, Utc};
use serde_json::Value;

use devicemapper::{CacheDev, Device, DmDevice, LinearDev, Sectors};

use crate::{
    engine::{
        strat_engine::{
            backstore::{
                blockdev::StratBlockDev,
                blockdevmgr::{map_to_dm, BlockDevMgr},
                cache_tier::CacheTier,
                data_tier::DataTier,
                devices::UnownedDevices,
                transaction::RequestTransaction,
            },
            dm::get_dm,
            metadata::MDADataSize,
            names::{format_backstore_ids, CacheRole},
            serde_structs::{BackstoreSave, CapSave, Recordable},
            writing::wipe_sectors,
        },
        types::{
            BlockDevTier, DevUuid, EncryptionInfo, KeyDescription, PoolEncryptionInfo, PoolUuid,
        },
    },
    stratis::{StratisError, StratisResult},
};

/// Use a cache block size that the kernel docs indicate is the largest
/// typical size.
const CACHE_BLOCK_SIZE: Sectors = Sectors(2048); // 1024 KiB

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

impl Backstore {
    /// Make a Backstore object from blockdevs that already belong to Stratis.
    /// Precondition: every device in datadevs and cachedevs has already been
    /// determined to belong to the pool with the specified pool_uuid.
    ///
    /// Precondition: backstore_save.cap.allocs[0].length <=
    ///       the sum of the lengths of the segments allocated
    /// to the data tier cap device.
    ///
    /// Precondition: backstore_save.data_segments is not empty. This is a
    /// consequence of the fact that metadata is saved by the pool, and if
    /// a pool exists, data has been allocated to the cap device.
    ///
    /// Precondition:
    ///   * key_description.is_some() -> every StratBlockDev in datadevs has a
    ///   key description and that key description == key_description
    ///   * key_description.is_none() -> no StratBlockDev in datadevs has a
    ///   key description.
    ///   * no StratBlockDev in cachedevs has a key description
    ///
    /// Postcondition:
    /// self.linear.is_some() XOR self.cache.is_some()
    /// self.cache.is_some() <=> self.cache_tier.is_some()
    pub fn setup(
        pool_uuid: PoolUuid,
        backstore_save: &BackstoreSave,
        datadevs: Vec<StratBlockDev>,
        cachedevs: Vec<StratBlockDev>,
        last_update_time: DateTime<Utc>,
    ) -> StratisResult<Backstore> {
        let block_mgr = BlockDevMgr::new(datadevs, Some(last_update_time));
        let data_tier = DataTier::setup(block_mgr, &backstore_save.data_tier)?;
        let (dm_name, dm_uuid) = format_backstore_ids(pool_uuid, CacheRole::OriginSub);
        let origin = LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            map_to_dm(&data_tier.segments),
        )?;

        let (cache_tier, cache, origin) = if !cachedevs.is_empty() {
            let block_mgr = BlockDevMgr::new(cachedevs, Some(last_update_time));
            match backstore_save.cache_tier {
                Some(ref cache_tier_save) => {
                    let cache_tier = CacheTier::setup(block_mgr, cache_tier_save)?;

                    let cache_device = make_cache(pool_uuid, &cache_tier, origin, false)?;
                    (Some(cache_tier), Some(cache_device), None)
                }
                None => {
                    let err_msg = "Cachedevs exist, but cache metdata does not exist";
                    return Err(StratisError::Msg(err_msg.into()));
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
    /// When the backstore is initialized it may be unencrypted, or it may
    /// be encrypted only with a kernel keyring and without Clevis information.
    ///
    /// WARNING: metadata changing event
    pub fn initialize(
        pool_uuid: PoolUuid,
        devices: UnownedDevices,
        mda_data_size: MDADataSize,
        encryption_info: Option<&EncryptionInfo>,
    ) -> StratisResult<Backstore> {
        let data_tier = DataTier::new(BlockDevMgr::initialize(
            pool_uuid,
            devices,
            mda_data_size,
            encryption_info,
        )?);

        Ok(Backstore {
            data_tier,
            cache_tier: None,
            linear: None,
            cache: None,
            next: Sectors(0),
        })
    }

    /// Initialize the cache tier and add cachedevs to the backstore.
    ///
    /// Returns all `DevUuid`s of devices that were added to the cache on initialization.
    ///
    /// Precondition: Must be invoked only after some space has been allocated
    /// from the backstore. This ensures that there is certainly a cap device.
    // Precondition: self.cache.is_none() && self.linear.is_some()
    // Postcondition: self.cache.is_some() && self.linear.is_none()
    pub fn init_cache(
        &mut self,
        pool_uuid: PoolUuid,
        devices: UnownedDevices,
    ) -> StratisResult<Vec<DevUuid>> {
        match self.cache_tier {
            Some(_) => unreachable!("self.cache.is_none()"),
            None => {
                // Note that variable length metadata is not stored on the
                // cachedevs, so the mda_size can always be the minimum.
                // If it is desired to change a cache dev to a data dev, it
                // should be removed and then re-added in order to ensure
                // that the MDA region is set to the correct size.
                let bdm =
                    BlockDevMgr::initialize(pool_uuid, devices, MDADataSize::default(), None)?;

                let cache_tier = CacheTier::new(bdm)?;

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

    /// Add cachedevs to the backstore.
    ///
    /// If the addition of the cache devs would result in a cache with a
    /// cache sub-device size greater than 32 TiB return an error.
    /// FIXME: This restriction on the size of the cache sub-device is
    /// expected to be removed in subsequent versions.
    ///
    /// Precondition: Must be invoked only after some space has been allocated
    /// from the backstore. This ensures that there is certainly a cap device.
    // Precondition: self.linear.is_none() && self.cache.is_some()
    // Precondition: self.cache_key_desc has the desired key description
    // Precondition: self.cache.is_some() && self.linear.is_none()
    pub fn add_cachedevs(
        &mut self,
        pool_uuid: PoolUuid,
        devices: UnownedDevices,
    ) -> StratisResult<Vec<DevUuid>> {
        match self.cache_tier {
            Some(ref mut cache_tier) => {
                let cache_device = self
                    .cache
                    .as_mut()
                    .expect("cache_tier.is_some() <=> self.cache.is_some()");
                let (uuids, (cache_change, meta_change)) = cache_tier.add(pool_uuid, devices)?;

                if cache_change {
                    let table = map_to_dm(&cache_tier.cache_segments);
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
            None => unreachable!("self.cache.is_some()"),
        }
    }

    /// Add datadevs to the backstore. The data tier always exists if the
    /// backstore exists at all, so there is no need to create it.
    pub fn add_datadevs(
        &mut self,
        pool_uuid: PoolUuid,
        devices: UnownedDevices,
    ) -> StratisResult<Vec<DevUuid>> {
        self.data_tier.add(pool_uuid, devices)
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
    pub fn request_alloc(
        &mut self,
        sizes: &[Sectors],
    ) -> StratisResult<Option<RequestTransaction>> {
        let mut transaction = match self.data_tier.alloc_request(sizes)? {
            Some(t) => t,
            None => return Ok(None),
        };

        let mut next = self.next;
        for size in sizes {
            transaction.add_seg_req((next, *size));
            next += *size
        }

        // Assert that the postcondition holds.
        assert_eq!(
            sizes,
            transaction
                .get_backstore()
                .iter()
                .map(|x| x.1)
                .collect::<Vec<Sectors>>()
                .as_slice()
        );

        Ok(Some(transaction))
    }

    /// Commit space requested by request_alloc() to metadata.
    ///
    /// This method commits the newly allocated data segments and then extends the cap device
    /// to be the same size as the allocated data size.
    pub fn commit_alloc(
        &mut self,
        pool_uuid: PoolUuid,
        transaction: RequestTransaction,
    ) -> StratisResult<()> {
        let segs = transaction.get_backstore();
        self.data_tier.alloc_commit(transaction)?;
        // This must occur after the segments have been updated in the data tier
        self.extend_cap_device(pool_uuid)?;

        assert!(self.next <= self.size());

        self.next += segs
            .into_iter()
            .fold(Sectors(0), |mut size, (_, next_size)| {
                size += next_size;
                size
            });

        Ok(())
    }

    /// Get only the datadevs in the pool.
    pub fn datadevs(&self) -> Vec<(DevUuid, &StratBlockDev)> {
        self.data_tier.blockdevs()
    }

    /// Get only the cachdevs in the pool.
    pub fn cachedevs(&self) -> Vec<(DevUuid, &StratBlockDev)> {
        match self.cache_tier {
            Some(ref cache) => cache.blockdevs(),
            None => Vec::new(),
        }
    }

    /// Return a reference to all the blockdevs that this pool has ownership
    /// of. The blockdevs may be returned in any order. It is unsafe to assume
    /// that they are grouped by tier or any other organization.
    pub fn blockdevs(&self) -> Vec<(DevUuid, BlockDevTier, &StratBlockDev)> {
        self.datadevs()
            .into_iter()
            .map(|(uuid, dev)| (uuid, BlockDevTier::Data, dev))
            .chain(
                self.cachedevs()
                    .into_iter()
                    .map(|(uuid, dev)| (uuid, BlockDevTier::Cache, dev)),
            )
            .collect()
    }

    pub fn blockdevs_mut(&mut self) -> Vec<(DevUuid, BlockDevTier, &mut StratBlockDev)> {
        match self.cache_tier {
            Some(ref mut cache) => cache
                .blockdevs_mut()
                .into_iter()
                .map(|(uuid, dev)| (uuid, BlockDevTier::Cache, dev))
                .chain(
                    self.data_tier
                        .blockdevs_mut()
                        .into_iter()
                        .map(|(uuid, dev)| (uuid, BlockDevTier::Data, dev)),
                )
                .collect(),
            None => self
                .data_tier
                .blockdevs_mut()
                .into_iter()
                .map(|(uuid, dev)| (uuid, BlockDevTier::Data, dev))
                .collect(),
        }
    }

    /// The current size of all the blockdevs in the data tier.
    pub fn datatier_size(&self) -> Sectors {
        self.data_tier.size()
    }

    /// The current size of allocated space on the blockdevs in the data tier.
    pub fn datatier_allocated_size(&self) -> Sectors {
        self.data_tier.allocated()
    }

    /// The current usable size of all the blockdevs in the data tier.
    pub fn datatier_usable_size(&self) -> Sectors {
        self.data_tier.usable_size()
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
    pub fn teardown(&mut self) -> StratisResult<()> {
        match self.cache {
            Some(ref mut cache) => cache.teardown(get_dm())?,
            None => {
                if let Some(ref mut linear) = self.linear {
                    linear.teardown(get_dm())?;
                }
            }
        };
        self.data_tier.block_mgr.teardown()
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

    /// The number of sectors in the backstore given up to Stratis metadata
    /// on devices in the data tier.
    pub fn datatier_metadata_size(&self) -> Sectors {
        self.data_tier.metadata_size()
    }

    /// Write the given data to the data tier's devices.
    pub fn save_state(&mut self, metadata: &[u8]) -> StratisResult<()> {
        self.data_tier.save_state(metadata)
    }

    /// Set user info field on the specified blockdev.
    /// May return an error if there is no blockdev for the given UUID.
    ///
    /// * Ok(Some(uuid)) provides the uuid of the changed blockdev
    /// * Ok(None) is returned if the blockdev was unchanged
    /// * Err(StratisError::Engine(_)) is returned if the UUID
    /// does not correspond to a blockdev
    pub fn set_blockdev_user_info(
        &mut self,
        uuid: DevUuid,
        user_info: Option<&str>,
    ) -> StratisResult<Option<DevUuid>> {
        self.get_mut_blockdev_by_uuid(uuid).map_or_else(
            || {
                Err(StratisError::Msg(format!(
                    "Blockdev with a UUID of {} was not found",
                    uuid
                )))
            },
            |(_, b)| {
                if b.set_user_info(user_info) {
                    Ok(Some(uuid))
                } else {
                    Ok(None)
                }
            },
        )
    }

    pub fn data_tier_is_encrypted(&self) -> bool {
        self.data_tier.block_mgr.is_encrypted()
    }

    pub fn data_tier_encryption_info(&self) -> Option<PoolEncryptionInfo> {
        self.data_tier.block_mgr.encryption_info()
    }

    pub fn has_cache(&self) -> bool {
        self.cache_tier.is_some()
    }

    pub fn bind_clevis(&mut self, pin: &str, clevis_info: &Value) -> StratisResult<bool> {
        self.data_tier.block_mgr.bind_clevis(pin, clevis_info)
    }

    pub fn unbind_clevis(&mut self) -> StratisResult<bool> {
        self.data_tier.block_mgr.unbind_clevis()
    }

    pub fn bind_keyring(&mut self, key_description: &KeyDescription) -> StratisResult<bool> {
        self.data_tier.block_mgr.bind_keyring(key_description)
    }

    pub fn unbind_keyring(&mut self) -> StratisResult<bool> {
        self.data_tier.block_mgr.unbind_keyring()
    }

    pub fn rebind_keyring(&mut self, new_key_desc: &KeyDescription) -> StratisResult<Option<bool>> {
        self.data_tier.block_mgr.rebind_keyring(new_key_desc)
    }

    pub fn rebind_clevis(&mut self) -> StratisResult<()> {
        self.data_tier.block_mgr.rebind_clevis()
    }

    pub fn grow(&mut self, dev: DevUuid) -> StratisResult<bool> {
        self.data_tier.grow(dev)
    }
}

impl<'a> Into<Value> for &'a Backstore {
    fn into(self) -> Value {
        json!({
            "blockdevs": {
                "datadevs": Value::Array(
                    self.datadevs().into_iter().map(|(_, dev)| {
                        dev.into()
                    }).collect()
                ),
                "cachedevs": Value::Array(
                    self.cachedevs().into_iter().map(|(_, dev)| {
                        dev.into()
                    }).collect()
                ),
            }
        })
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
    use std::{collections::HashSet, fs::OpenOptions, path::Path};

    use devicemapper::{CacheDevStatus, DataBlocks, DmOptions, IEC};

    use crate::engine::strat_engine::{
        backstore::process_and_verify_devices,
        metadata::device_identifiers,
        tests::{loopbacked, real},
    };

    use super::*;

    const INITIAL_BACKSTORE_ALLOCATION: Sectors = CACHE_BLOCK_SIZE;

    /// Assert some invariants of the backstore
    /// * backstore.cache_tier.is_some() <=> backstore.cache.is_some() &&
    ///   backstore.cache_tier.is_some() => backstore.linear.is_none()
    /// * backstore's data tier allocated is equal to the size of the cap device
    /// * backstore's next index is always less than the size of the cap
    ///   device
    fn invariant(backstore: &Backstore) {
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
    fn test_add_cache_devs(paths: &[&Path]) {
        assert!(paths.len() > 3);

        let meta_size = Sectors(IEC::Mi);

        let (initcachepaths, paths) = paths.split_at(1);
        let (cachedevpaths, paths) = paths.split_at(1);
        let (datadevpaths, initdatapaths) = paths.split_at(1);

        let pool_uuid = PoolUuid::new_v4();

        let datadevs =
            process_and_verify_devices(pool_uuid, &HashSet::new(), datadevpaths).unwrap();
        let cachedevs =
            process_and_verify_devices(pool_uuid, &HashSet::new(), cachedevpaths).unwrap();

        let initdatadevs =
            process_and_verify_devices(pool_uuid, &HashSet::new(), initdatapaths).unwrap();

        let initcachedevs =
            process_and_verify_devices(pool_uuid, &HashSet::new(), initcachepaths).unwrap();

        let mut backstore =
            Backstore::initialize(pool_uuid, initdatadevs, MDADataSize::default(), None).unwrap();

        invariant(&backstore);

        // Allocate space from the backstore so that the cap device is made.
        let transaction = backstore
            .request_alloc(&[INITIAL_BACKSTORE_ALLOCATION])
            .unwrap()
            .unwrap();
        backstore.commit_alloc(pool_uuid, transaction).unwrap();

        let cache_uuids = backstore.init_cache(pool_uuid, initcachedevs).unwrap();

        invariant(&backstore);

        assert_eq!(cache_uuids.len(), initcachepaths.len());
        assert_matches!(backstore.linear, None);

        let cache_status = backstore
            .cache
            .as_ref()
            .map(|c| c.status(get_dm(), DmOptions::default()).unwrap())
            .unwrap();

        match cache_status {
            CacheDevStatus::Working(status) => {
                let usage = &status.usage;
                assert_eq!(usage.used_cache, DataBlocks(0));
                assert_eq!(usage.total_meta, meta_size.metablocks());
                assert!(usage.total_cache > DataBlocks(0));
            }
            CacheDevStatus::Error => panic!("cache status could not be obtained"),
            CacheDevStatus::Fail => panic!("cache is in a failed state"),
        }

        let data_uuids = backstore.add_datadevs(pool_uuid, datadevs).unwrap();
        invariant(&backstore);
        assert_eq!(data_uuids.len(), datadevpaths.len());

        let cache_uuids = backstore.add_cachedevs(pool_uuid, cachedevs).unwrap();
        invariant(&backstore);
        assert_eq!(cache_uuids.len(), cachedevpaths.len());

        let cache_status = backstore
            .cache
            .as_ref()
            .map(|c| c.status(get_dm(), DmOptions::default()).unwrap())
            .unwrap();

        match cache_status {
            CacheDevStatus::Working(status) => {
                let usage = &status.usage;
                assert_eq!(usage.used_cache, DataBlocks(0));
                assert_eq!(usage.total_meta, meta_size.metablocks());
                assert!(usage.total_cache > DataBlocks(0));
            }
            CacheDevStatus::Error => panic!("cache status could not be obtained"),
            CacheDevStatus::Fail => panic!("cache is in a failed state"),
        }

        backstore.destroy().unwrap();
    }

    #[test]
    fn loop_test_add_cache_devs() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(4, 5, None),
            test_add_cache_devs,
        );
    }

    #[test]
    fn real_test_add_cache_devs() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(4, None, None),
            test_add_cache_devs,
        );
    }

    /// Create a backstore.
    /// Initialize a cache and verify that there is a new device representing
    /// the cache.
    fn test_setup(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let pool_uuid = PoolUuid::new_v4();

        let devices1 = process_and_verify_devices(pool_uuid, &HashSet::new(), paths1).unwrap();
        let devices2 = process_and_verify_devices(pool_uuid, &HashSet::new(), paths2).unwrap();

        let mut backstore =
            Backstore::initialize(pool_uuid, devices1, MDADataSize::default(), None).unwrap();

        for path in paths1 {
            assert_eq!(
                pool_uuid,
                device_identifiers(&mut OpenOptions::new().read(true).open(path).unwrap())
                    .unwrap()
                    .unwrap()
                    .pool_uuid
            );
        }

        invariant(&backstore);

        // Allocate space from the backstore so that the cap device is made.
        let transaction = backstore
            .request_alloc(&[INITIAL_BACKSTORE_ALLOCATION])
            .unwrap()
            .unwrap();
        backstore.commit_alloc(pool_uuid, transaction).unwrap();

        let old_device = backstore.device();

        backstore.init_cache(pool_uuid, devices2).unwrap();

        for path in paths2 {
            assert_eq!(
                pool_uuid,
                device_identifiers(&mut OpenOptions::new().read(true).open(path).unwrap())
                    .unwrap()
                    .unwrap()
                    .pool_uuid
            );
        }

        invariant(&backstore);

        assert_ne!(backstore.device(), old_device);

        backstore.destroy().unwrap();
    }

    #[test]
    fn loop_test_setup() {
        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Range(2, 3, None), test_setup);
    }

    #[test]
    fn real_test_setup() {
        real::test_with_spec(&real::DeviceLimits::AtLeast(2, None, None), test_setup);
    }
}

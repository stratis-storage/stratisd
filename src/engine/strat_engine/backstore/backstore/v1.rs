// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle the backing store of a pool.

use std::{cmp, collections::HashMap, fs, path::PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;
use tempfile::TempDir;

use devicemapper::{CacheDev, Device, DmDevice, LinearDev, Sectors};

use crate::{
    engine::{
        shared::gather_encryption_info,
        strat_engine::{
            backstore::{
                backstore::InternalBackstore,
                blockdev::{v1::StratBlockDev, InternalBlockDev},
                blockdevmgr::BlockDevMgr,
                cache_tier::CacheTier,
                data_tier::DataTier,
                devices::UnownedDevices,
                shared::BlockSizeSummary,
            },
            crypt::{back_up_luks_header, handle::v1::CryptHandle, restore_luks_header},
            dm::{get_dm, list_of_backstore_devices, remove_optional_devices},
            metadata::{MDADataSize, BDA},
            names::{format_backstore_ids, CacheRole},
            serde_structs::{BackstoreSave, CapSave, Recordable},
            shared::bds_to_bdas,
            types::BDARecordResult,
            writing::wipe_sectors,
        },
        types::{
            ActionAvailability, BlockDevTier, DevUuid, EncryptionInfo, InputEncryptionInfo,
            KeyDescription, Name, PoolEncryptionInfo, PoolUuid,
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
    cache_tier: &CacheTier<StratBlockDev>,
    origin: LinearDev,
    new: bool,
) -> StratisResult<CacheDev> {
    let (dm_name, dm_uuid) = format_backstore_ids(pool_uuid, CacheRole::MetaSub);
    let meta = LinearDev::setup(
        get_dm(),
        &dm_name,
        Some(&dm_uuid),
        cache_tier.meta_segments.map_to_dm(),
    )?;

    if new {
        // See comment in ThinPool::new() method
        wipe_sectors(
            meta.devnode(),
            Sectors(0),
            cmp::min(Sectors(8), meta.size()),
        )?;
    }

    let (dm_name, dm_uuid) = format_backstore_ids(pool_uuid, CacheRole::CacheSub);
    let cache = LinearDev::setup(
        get_dm(),
        &dm_name,
        Some(&dm_uuid),
        cache_tier.cache_segments.map_to_dm(),
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
    cache_tier: Option<CacheTier<StratBlockDev>>,
    /// Coordinates handling of the blockdevs that form the base.
    data_tier: DataTier<StratBlockDev>,
    /// A linear DM device.
    linear: Option<LinearDev>,
    /// Index for managing allocation of cap device
    next: Sectors,
}

impl InternalBackstore for Backstore {
    fn device(&self) -> Option<Device> {
        self.cache
            .as_ref()
            .map(|d| d.device())
            .or_else(|| self.linear.as_ref().map(|d| d.device()))
    }

    fn datatier_allocated_size(&self) -> Sectors {
        self.data_tier.allocated()
    }

    fn datatier_usable_size(&self) -> Sectors {
        self.data_tier.usable_size()
    }

    fn available_in_backstore(&self) -> Sectors {
        self.data_tier.usable_size() - self.next
    }

    fn alloc(
        &mut self,
        pool_uuid: PoolUuid,
        sizes: &[Sectors],
    ) -> StratisResult<Option<Vec<(Sectors, Sectors)>>> {
        let total_required = sizes.iter().cloned().sum();
        if self.available_in_backstore() < total_required {
            return Ok(None);
        }

        if self.data_tier.alloc(sizes) {
            self.extend_cap_device(pool_uuid)?;
        } else {
            return Ok(None);
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
    ///     key description and that key description == key_description
    ///   * key_description.is_none() -> no StratBlockDev in datadevs has a
    ///     key description.
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
    ) -> BDARecordResult<Backstore> {
        let block_mgr = BlockDevMgr::new(datadevs, Some(last_update_time));
        let data_tier = DataTier::setup(block_mgr, &backstore_save.data_tier)?;
        let (dm_name, dm_uuid) = format_backstore_ids(pool_uuid, CacheRole::OriginSub);
        let origin = match LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            data_tier.segments.map_to_dm(),
        ) {
            Ok(origin) => origin,
            Err(e) => {
                return Err((
                    StratisError::from(e),
                    data_tier
                        .block_mgr
                        .into_bdas()
                        .into_iter()
                        .chain(bds_to_bdas(cachedevs))
                        .collect::<HashMap<_, _>>(),
                ));
            }
        };

        let (cache_tier, cache, origin) = if !cachedevs.is_empty() {
            let block_mgr = BlockDevMgr::new(cachedevs, Some(last_update_time));
            match backstore_save.cache_tier {
                Some(ref cache_tier_save) => {
                    let cache_tier = match CacheTier::setup(block_mgr, cache_tier_save) {
                        Ok(ct) => ct,
                        Err((e, mut bdas)) => {
                            bdas.extend(data_tier.block_mgr.into_bdas());
                            return Err((e, bdas));
                        }
                    };

                    let cache_device = match make_cache(pool_uuid, &cache_tier, origin, false) {
                        Ok(cd) => cd,
                        Err(e) => {
                            return Err((
                                e,
                                data_tier
                                    .block_mgr
                                    .into_bdas()
                                    .into_iter()
                                    .chain(cache_tier.block_mgr.into_bdas())
                                    .collect::<HashMap<_, _>>(),
                            ));
                        }
                    };
                    (Some(cache_tier), Some(cache_device), None)
                }
                None => {
                    let err_msg = "Cachedevs exist, but cache metadata does not exist";
                    return Err((
                        StratisError::Msg(err_msg.into()),
                        data_tier
                            .block_mgr
                            .into_bdas()
                            .into_iter()
                            .chain(block_mgr.into_bdas())
                            .collect::<HashMap<_, _>>(),
                    ));
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
    #[cfg(any(test, feature = "extras"))]
    pub fn initialize(
        pool_name: Name,
        pool_uuid: PoolUuid,
        devices: UnownedDevices,
        mda_data_size: MDADataSize,
        encryption_info: Option<&InputEncryptionInfo>,
    ) -> StratisResult<Backstore> {
        let data_tier = DataTier::<StratBlockDev>::new(BlockDevMgr::<StratBlockDev>::initialize(
            pool_name,
            pool_uuid,
            devices,
            mda_data_size,
            encryption_info,
            None,
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
        pool_name: Name,
        pool_uuid: PoolUuid,
        devices: UnownedDevices,
        sector_size: Option<u32>,
    ) -> StratisResult<Vec<DevUuid>> {
        match self.cache_tier {
            Some(_) => unreachable!("self.cache.is_none()"),
            None => {
                // Note that variable length metadata is not stored on the
                // cachedevs, so the mda_size can always be the minimum.
                // If it is desired to change a cache dev to a data dev, it
                // should be removed and then re-added in order to ensure
                // that the MDA region is set to the correct size.
                let bdm = BlockDevMgr::<StratBlockDev>::initialize(
                    pool_name,
                    pool_uuid,
                    devices,
                    MDADataSize::default(),
                    self.encryption_info()
                        .map(EncryptionInfo::try_from)
                        .transpose()?
                        .map(InputEncryptionInfo::from)
                        .as_ref(),
                    sector_size,
                )?;

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
        pool_name: Name,
        pool_uuid: PoolUuid,
        devices: UnownedDevices,
        sector_size: Option<u32>,
    ) -> StratisResult<Vec<DevUuid>> {
        match self.cache_tier {
            Some(ref mut cache_tier) => {
                let cache_device = self
                    .cache
                    .as_mut()
                    .expect("cache_tier.is_some() <=> self.cache.is_some()");
                let (uuids, (cache_change, meta_change)) =
                    cache_tier.add(pool_name, pool_uuid, devices, sector_size)?;

                if cache_change {
                    let table = cache_tier.cache_segments.map_to_dm();
                    cache_device.set_cache_table(get_dm(), table)?;
                    cache_device.resume(get_dm())?;
                }

                // NOTE: currently CacheTier::add() does not ever update the
                // meta segments. That means that this code is dead. But,
                // when CacheTier::add() is fixed, this code will become live.
                if meta_change {
                    let table = cache_tier.meta_segments.map_to_dm();
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
        pool_name: Name,
        pool_uuid: PoolUuid,
        devices: UnownedDevices,
        sector_size: Option<u32>,
    ) -> StratisResult<Vec<DevUuid>> {
        self.data_tier
            .add(pool_name, pool_uuid, devices, sector_size)
    }

    /// Extend the cap device whether it is a cache or not. Create the DM
    /// device if it does not already exist. Return an error if DM
    /// operations fail. Use all segments currently allocated in the data tier.
    fn extend_cap_device(&mut self, pool_uuid: PoolUuid) -> StratisResult<()> {
        let create = match (self.cache.as_mut(), self.linear.as_mut()) {
            (None, None) => true,
            (Some(cache), None) => {
                let table = self.data_tier.segments.map_to_dm();
                cache.set_origin_table(get_dm(), table)?;
                cache.resume(get_dm())?;
                false
            }
            (None, Some(linear)) => {
                let table = self.data_tier.segments.map_to_dm();
                linear.set_table(get_dm(), table)?;
                linear.resume(get_dm())?;
                false
            }
            _ => panic!("NOT (self.cache().is_some() AND self.linear.is_some())"),
        };

        if create {
            let table = self.data_tier.segments.map_to_dm();
            let (dm_name, dm_uuid) = format_backstore_ids(pool_uuid, CacheRole::OriginSub);
            let origin = LinearDev::setup(get_dm(), &dm_name, Some(&dm_uuid), table)?;
            self.linear = Some(origin);
        }

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

    /// The size of the cap device.
    ///
    /// The size of the cap device is obtained from the size of the component
    /// DM devices. But the devicemapper library stores the data from which
    /// the size of each DM device is calculated; the result is computed and
    /// no ioctl is required.
    #[cfg(test)]
    fn size(&self) -> Sectors {
        self.linear
            .as_ref()
            .map(|d| d.size())
            .or_else(|| self.cache.as_ref().map(|d| d.size()))
            .unwrap_or(Sectors(0))
    }

    /// Destroy the entire store.
    pub fn destroy(&mut self, pool_uuid: PoolUuid) -> StratisResult<()> {
        let devs = list_of_backstore_devices(pool_uuid);
        remove_optional_devices(devs)?;
        if let Some(ref mut cache_tier) = self.cache_tier {
            cache_tier.destroy()?;
        }
        self.data_tier.destroy()
    }

    /// Teardown the DM devices in the backstore.
    pub fn teardown(&mut self, pool_uuid: PoolUuid) -> StratisResult<()> {
        let devs = list_of_backstore_devices(pool_uuid);
        remove_optional_devices(devs)?;
        if let Some(ref mut cache_tier) = self.cache_tier {
            cache_tier.block_mgr.teardown()?;
        }
        self.data_tier.block_mgr.teardown()
    }

    /// Consume the backstore and convert it into a set of BDAs representing
    /// all data and cache devices.
    pub fn into_bdas(self) -> HashMap<DevUuid, BDA> {
        self.data_tier
            .block_mgr
            .into_bdas()
            .into_iter()
            .chain(
                self.cache_tier
                    .map(|ct| ct.block_mgr.into_bdas())
                    .unwrap_or_default(),
            )
            .collect::<HashMap<_, _>>()
    }

    /// Drain the backstore devices into a set of all data and cache devices.
    pub fn drain_bds(&mut self) -> Vec<StratBlockDev> {
        let mut bds = self.data_tier.block_mgr.drain_bds();
        bds.extend(
            self.cache_tier
                .as_mut()
                .map(|ct| ct.block_mgr.drain_bds())
                .unwrap_or_default(),
        );
        bds
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

    /// Read the currently saved state from the data tier's devices.
    pub fn load_state(&self) -> StratisResult<Vec<u8>> {
        self.data_tier.load_state()
    }

    /// Set user info field on the specified blockdev.
    /// May return an error if there is no blockdev for the given UUID.
    ///
    /// * Ok(Some(uuid)) provides the uuid of the changed blockdev
    /// * Ok(None) is returned if the blockdev was unchanged
    /// * Err(StratisError::Engine(_)) is returned if the UUID
    ///   does not correspond to a blockdev
    pub fn set_blockdev_user_info(
        &mut self,
        uuid: DevUuid,
        user_info: Option<&str>,
    ) -> StratisResult<Option<DevUuid>> {
        self.get_mut_blockdev_by_uuid(uuid).map_or_else(
            || {
                Err(StratisError::Msg(format!(
                    "Blockdev with a UUID of {uuid} was not found"
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

    pub fn is_encrypted(&self) -> bool {
        if let Some(ref ct) = self.cache_tier {
            assert_eq!(
                self.data_tier.block_mgr.is_encrypted(),
                ct.block_mgr.is_encrypted()
            );
        }
        self.data_tier.block_mgr.is_encrypted()
    }

    pub fn has_cache(&self) -> bool {
        self.cache_tier.is_some()
    }

    /// Gather the encryption information for all block devices in the backstore.
    pub fn encryption_info(&self) -> Option<PoolEncryptionInfo> {
        let blockdevs = self.blockdevs();
        gather_encryption_info(
            blockdevs.len(),
            blockdevs.iter().map(|(_, _, bd)| bd.encryption_info()),
        )
        .expect("All devices must be either encrypted or unencrypted for the pool to be set up")
    }

    /// Bind all devices in the given backstore using the given clevis
    /// configuration.
    ///
    /// * Returns Ok(true) if the binding was performed.
    /// * Returns Ok(false) if the binding had already been previously performed and
    ///   nothing was changed.
    /// * Returns Err(_) if an inconsistency was found in the metadata across pools
    ///   or binding failed.
    pub fn bind_clevis(&mut self, pin: &str, clevis_info: &Value) -> StratisResult<bool> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ));
            }
        };

        if let Some((_, (ref existing_pin, ref existing_info))) =
            encryption_info.single_clevis_info()
        {
            if existing_pin.as_str() == pin
                && CryptHandle::can_unlock(
                    self.blockdevs()
                        .first()
                        .expect("Must have at least one blockdev")
                        .2
                        .physical_path(),
                    false,
                    true,
                )
            {
                Ok(false)
            } else {
                Err(StratisError::Msg(format!(
                    "Block devices have already been bound with pin {existing_pin} and config {existing_info}; \
                        requested pin {pin} and config {clevis_info} can't be applied"
                )))
            }
        } else {
            operation_loop(
                self.blockdevs_mut().into_iter().map(|(_, _, bd)| bd),
                |blockdev| blockdev.bind_clevis(pin, clevis_info),
            )?;
            Ok(true)
        }
    }

    /// Unbind all devices in the given backstore from clevis.
    ///
    /// * Returns Ok(true) if the unbinding was performed.
    /// * Returns Ok(false) if the unbinding had already been previously performed and
    ///   nothing was changed.
    /// * Returns Err(_) if an inconsistency was found in the metadata across pools
    ///   or unbinding failed.
    pub fn unbind_clevis(&mut self) -> StratisResult<bool> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ));
            }
        };

        if encryption_info.single_clevis_info().is_some() {
            operation_loop(
                self.blockdevs_mut().into_iter().map(|(_, _, bd)| bd),
                |blockdev| blockdev.unbind_clevis(),
            )?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Bind all devices in the given backstore to a passphrase using the
    /// given key description.
    ///
    /// * Returns Ok(true) if the binding was performed.
    /// * Returns Ok(false) if the binding had already been previously performed and
    ///   nothing was changed.
    /// * Returns Err(_) if an inconsistency was found in the metadata across pools
    ///   or binding failed.
    pub fn bind_keyring(&mut self, key_desc: &KeyDescription) -> StratisResult<bool> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ));
            }
        };

        if let Some((_, kd)) = encryption_info.single_key_description() {
            if kd == key_desc {
                if CryptHandle::can_unlock(
                    self.blockdevs()
                        .first()
                        .expect("Must have at least one blockdev")
                        .2
                        .physical_path(),
                    true,
                    false,
                ) {
                    Ok(false)
                } else {
                    Err(StratisError::Msg(format!(
                        "Key description {} is registered in the metadata but the \
                            associated passphrase can't unlock the device; the \
                            associated passphrase may have changed since binding",
                        key_desc.as_application_str(),
                    )))
                }
            } else {
                Err(StratisError::Msg(format!(
                    "Block devices have already been bound with key description {}; \
                        requested key description {} can't be applied",
                    kd.as_application_str(),
                    key_desc.as_application_str(),
                )))
            }
        } else {
            operation_loop(
                self.blockdevs_mut().into_iter().map(|(_, _, bd)| bd),
                |blockdev| blockdev.bind_keyring(key_desc),
            )?;
            Ok(true)
        }
    }

    /// Unbind all devices in the given backstore from the passphrase
    /// associated with the key description.
    ///
    /// * Returns Ok(true) if the unbinding was performed.
    /// * Returns Ok(false) if the unbinding had already been previously performed and
    ///   nothing was changed.
    /// * Returns Err(_) if an inconsistency was found in the metadata across pools
    ///   or unbinding failed.
    pub fn unbind_keyring(&mut self) -> StratisResult<bool> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ));
            }
        };

        if encryption_info.single_key_description().is_some() {
            operation_loop(
                self.blockdevs_mut().into_iter().map(|(_, _, bd)| bd),
                |blockdev| blockdev.unbind_keyring(),
            )?;
            Ok(true)
        } else {
            // is encrypted and key description is None
            Ok(false)
        }
    }

    /// Change the keyring passphrase associated with the block devices in
    /// this pool.
    ///
    /// Returns:
    /// * Ok(None) if the pool is not currently bound to a keyring passphrase.
    /// * Ok(Some(true)) if the pool was successfully bound to the new key description.
    /// * Ok(Some(false)) if the pool is already bound to this key description.
    /// * Err(_) if an operation fails while changing the passphrase.
    pub fn rebind_keyring(&mut self, key_desc: &KeyDescription) -> StratisResult<Option<bool>> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ));
            }
        };

        if encryption_info.single_key_description().map(|(_, kd)| kd) == Some(key_desc) {
            Ok(Some(false))
        } else if encryption_info.single_key_description().is_some() {
            // Keys are not the same but key description is present
            operation_loop(
                self.blockdevs_mut().into_iter().map(|(_, _, bd)| bd),
                |blockdev| blockdev.rebind_keyring(key_desc),
            )?;
            Ok(Some(true))
        } else {
            Ok(None)
        }
    }

    /// Reencrypt all encrypted devices in the pool.
    ///
    /// Returns:
    /// * Ok(()) if successful
    /// * Err(_) if an operation fails while reencrypting the devices.
    pub fn reencrypt(&mut self) -> StratisResult<()> {
        if self.encryption_info().is_none() {
            return Err(StratisError::Msg(
                "Requested pool does not appear to be encrypted".to_string(),
            ));
        };

        // Keys are not the same but key description is present
        operation_loop(
            self.blockdevs_mut().into_iter().map(|(_, _, bd)| bd),
            |blockdev| blockdev.reencrypt(),
        )?;
        Ok(())
    }

    /// Regenerate the Clevis bindings with the block devices in this pool using
    /// the same configuration.
    ///
    /// The method for this rollback caches the initial Clevis metadata and
    /// reverts all of the devices if there is a failure.
    ///
    /// This method returns StratisResult<()> because the Clevis regen command
    /// will always change the metadata when successful. The command is not idempotent
    /// so this method will either fail to regenerate the bindings or it will
    /// result in a metadata change.
    pub fn rebind_clevis(&mut self) -> StratisResult<()> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ));
            }
        };

        if encryption_info.single_clevis_info().is_none() {
            Err(StratisError::Msg(
                "Requested pool is not already bound to Clevis".to_string(),
            ))
        } else {
            operation_loop(
                self.blockdevs_mut().into_iter().map(|(_, _, bd)| bd),
                |blockdev| blockdev.rebind_clevis(),
            )?;

            Ok(())
        }
    }

    pub fn grow(&mut self, dev: DevUuid) -> StratisResult<bool> {
        self.data_tier.grow(dev)
    }

    /// Rename pool name in LUKS2 token if pool is encrypted.
    pub fn rename_pool(&mut self, new_name: &Name) -> StratisResult<()> {
        if self.encryption_info().is_some() {
            operation_loop(
                self.blockdevs_mut().into_iter().map(|(_, _, bd)| bd),
                |blockdev| blockdev.rename_pool(new_name.clone()),
            )?;
        }
        Ok(())
    }

    /// A summary of block sizes
    pub fn block_size_summary(&self, tier: BlockDevTier) -> Option<BlockSizeSummary> {
        match tier {
            BlockDevTier::Data => Some(self.data_tier.partition_by_use().into()),
            BlockDevTier::Cache => self
                .cache_tier
                .as_ref()
                .map(|ct| ct.partition_cache_by_use().into()),
        }
    }

    /// What the pool's action availability should be
    pub fn action_availability(&self) -> ActionAvailability {
        let data_tier_bs_summary = self
            .block_size_summary(BlockDevTier::Data)
            .expect("always exists");
        let cache_tier_bs_summary: Option<BlockSizeSummary> =
            self.block_size_summary(BlockDevTier::Cache);
        if let Err(err) = data_tier_bs_summary.validate() {
            warn!("Disabling pool changes for this pool: {}", err);
            ActionAvailability::NoPoolChanges
        } else if let Some(Err(err)) = cache_tier_bs_summary.map(|ct| ct.validate()) {
            // NOTE: This condition should be impossible. Since the cache is
            // always expanded to include all its devices, and an attempt to add
            // more devices than the cache can use causes the devices to be
            // rejected, there should be no unused devices in a cache. If, for
            // some reason this condition fails, though, NoPoolChanges would
            // be the correct state to put the pool in.
            warn!("Disabling pool changes for this pool: {}", err);
            ActionAvailability::NoPoolChanges
        } else {
            ActionAvailability::Full
        }
    }
}

impl Into<Value> for &Backstore {
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
                crypt_meta_allocs: Vec::new(),
            },
            data_tier: self.data_tier.record(),
        }
    }
}

fn operation_loop<'a, I, A>(blockdevs: I, action: A) -> StratisResult<()>
where
    I: IntoIterator<Item = &'a mut StratBlockDev>,
    A: Fn(&mut StratBlockDev) -> StratisResult<()>,
{
    fn rollback_loop(
        rollback_record: Vec<&mut StratBlockDev>,
        headers: Vec<PathBuf>,
        causal_error: StratisError,
    ) -> StratisError {
        // NOTE: Zip can be used here because the header will always be backed up before
        // the operation is performed. As a result, the header iterator will always be
        // equal to or longer than the blockdev record iterator which means all blockdevs
        // that have had operations performed on them will always be restored.
        for (blockdev, header) in rollback_record.into_iter().zip(headers) {
            if let Err(e) = restore_luks_header(blockdev.devnode(), header.as_path()) {
                warn!(
                    "Failed to roll back device operation for device {}: {}",
                    blockdev.physical_path().display(),
                    e,
                );
                return StratisError::RollbackError {
                    causal_error: Box::new(causal_error),
                    rollback_error: Box::new(e),
                    level: ActionAvailability::NoRequests,
                };
            }
            if let Err(e) = blockdev.reload_crypt_metadata() {
                warn!(
                    "Failed to reload on-disk metadata for device {}: {}",
                    blockdev.physical_path().display(),
                    e,
                );
                return StratisError::RollbackError {
                    causal_error: Box::new(causal_error),
                    rollback_error: Box::new(e),
                    level: ActionAvailability::NoRequests,
                };
            }
        }

        causal_error
    }

    fn perform_operation<'a, I, A>(tmp_dir: &TempDir, blockdevs: I, action: A) -> StratisResult<()>
    where
        I: IntoIterator<Item = &'a mut StratBlockDev>,
        A: Fn(&mut StratBlockDev) -> StratisResult<()>,
    {
        let mut original_headers = Vec::new();
        let mut rollback_record = Vec::new();
        for blockdev in blockdevs {
            match back_up_luks_header(blockdev.physical_path(), tmp_dir) {
                Ok(h) => original_headers.push(h),
                Err(e) => return Err(rollback_loop(rollback_record, original_headers, e)),
            };
            let res = action(blockdev);
            rollback_record.push(blockdev);
            if let Err(error) = res {
                return Err(rollback_loop(rollback_record, original_headers, error));
            }
        }

        Ok(())
    }

    let tmp_dir = TempDir::new()?;
    let res = perform_operation(&tmp_dir, blockdevs, action);
    if let Err(e) = fs::remove_dir_all(tmp_dir.path()) {
        warn!(
            "Leaked temporary files at path {}: {}",
            tmp_dir.path().display(),
            e
        );
    }
    res
}

#[cfg(test)]
mod tests {
    use std::{env, fs::OpenOptions, path::Path};

    use devicemapper::{CacheDevStatus, DataBlocks, DmOptions, IEC};

    use crate::engine::strat_engine::{
        backstore::devices::{ProcessedPathInfos, UnownedDevices},
        cmd,
        metadata::device_identifiers,
        ns::{unshare_mount_namespace, MemoryFilesystem},
        tests::{crypt, loopbacked, real},
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
                (&None, Some(cache)) => cache.size(),
                (Some(linear), &None) => linear.size(),
                _ => panic!("impossible; see first assertion"),
            }
        );
        assert!(backstore.next <= backstore.size());

        backstore.data_tier.invariant();

        if let Some(cache_tier) = &backstore.cache_tier {
            cache_tier.invariant()
        }
    }

    fn get_devices(paths: &[&Path]) -> StratisResult<UnownedDevices> {
        ProcessedPathInfos::try_from(paths)
            .map(|ps| ps.unpack())
            .map(|(sds, uds)| {
                sds.error_on_not_empty().unwrap();
                uds
            })
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

        let datadevs = get_devices(datadevpaths).unwrap();
        let cachedevs = get_devices(cachedevpaths).unwrap();
        let initdatadevs = get_devices(initdatapaths).unwrap();
        let initcachedevs = get_devices(initcachepaths).unwrap();

        let pool_name = Name::new("pool_name".to_string());
        let mut backstore = Backstore::initialize(
            pool_name.clone(),
            pool_uuid,
            initdatadevs,
            MDADataSize::default(),
            None,
        )
        .unwrap();

        invariant(&backstore);

        // Allocate space from the backstore so that the cap device is made.
        backstore
            .alloc(pool_uuid, &[INITIAL_BACKSTORE_ALLOCATION])
            .unwrap()
            .unwrap();

        let cache_uuids = backstore
            .init_cache(pool_name.clone(), pool_uuid, initcachedevs, None)
            .unwrap();

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

        let data_uuids = backstore
            .add_datadevs(pool_name.clone(), pool_uuid, datadevs, None)
            .unwrap();
        invariant(&backstore);
        assert_eq!(data_uuids.len(), datadevpaths.len());

        let cache_uuids = backstore
            .add_cachedevs(pool_name, pool_uuid, cachedevs, None)
            .unwrap();
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

        backstore.destroy(pool_uuid).unwrap();
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
        let pool_name = Name::new("pool_name".to_string());

        let devices1 = get_devices(paths1).unwrap();
        let devices2 = get_devices(paths2).unwrap();

        let mut backstore = Backstore::initialize(
            pool_name.clone(),
            pool_uuid,
            devices1,
            MDADataSize::default(),
            None,
        )
        .unwrap();

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
        backstore
            .alloc(pool_uuid, &[INITIAL_BACKSTORE_ALLOCATION])
            .unwrap()
            .unwrap();

        let old_device = backstore.device();

        backstore
            .init_cache(pool_name, pool_uuid, devices2, None)
            .unwrap();

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

        backstore.destroy(pool_uuid).unwrap();
    }

    #[test]
    fn loop_test_setup() {
        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Range(2, 3, None), test_setup);
    }

    #[test]
    fn real_test_setup() {
        real::test_with_spec(&real::DeviceLimits::AtLeast(2, None, None), test_setup);
    }

    fn test_clevis_initialize(paths: &[&Path]) {
        unshare_mount_namespace().unwrap();

        let pool_name = Name::new("pool_name".to_string());
        let _memfs = MemoryFilesystem::new().unwrap();
        let pool_uuid = PoolUuid::new_v4();
        let mut backstore = Backstore::initialize(
            pool_name,
            pool_uuid,
            get_devices(paths).unwrap(),
            MDADataSize::default(),
            InputEncryptionInfo::new_legacy(None, Some((
                "tang".to_string(),
                json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
            ))).as_ref()
        )
        .unwrap();
        cmd::udev_settle().unwrap();

        assert_matches!(
            backstore.bind_clevis(
                "tang",
                &json!({"url": env::var("TANG_URL").expect("TANG_URL env var required")})
            ),
            Ok(false)
        );

        assert_matches!(
            backstore.bind_clevis(
                "tang",
                &json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true})
            ),
            Ok(false)
        );

        invariant(&backstore);
    }

    #[test]
    fn clevis_real_test_initialize() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_clevis_initialize,
        );
    }

    #[test]
    #[should_panic]
    fn clevis_real_should_fail_test_initialize() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_clevis_initialize,
        );
    }

    #[test]
    fn clevis_loop_test_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 4, None),
            test_clevis_initialize,
        );
    }

    #[test]
    #[should_panic]
    fn clevis_loop_should_fail_test_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 4, None),
            test_clevis_initialize,
        );
    }

    fn test_clevis_both_initialize(paths: &[&Path]) {
        fn test_both(paths: &[&Path], key_desc: &KeyDescription) {
            unshare_mount_namespace().unwrap();

            let _memfs = MemoryFilesystem::new().unwrap();
            let pool_uuid = PoolUuid::new_v4();
            let pool_name = Name::new("pool_name".to_string());
            let mut backstore = Backstore::initialize(
                pool_name,
                pool_uuid,
                get_devices(paths).unwrap(),
                MDADataSize::default(),
                InputEncryptionInfo::new_legacy(
                    Some(key_desc.clone()),
                    Some((
                        "tang".to_string(),
                        json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
                    )),
                ).as_ref(),
            ).unwrap();
            cmd::udev_settle().unwrap();

            if backstore.bind_clevis(
                "tang",
                &json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
            ).unwrap() {
                panic!(
                    "Clevis bind idempotence test failed"
                );
            }

            invariant(&backstore);

            if backstore.bind_keyring(key_desc).unwrap() {
                panic!("Keyring bind idempotence test failed")
            }

            invariant(&backstore);

            if !backstore.unbind_clevis().unwrap() {
                panic!("Clevis unbind test failed");
            }

            invariant(&backstore);

            if backstore.unbind_clevis().unwrap() {
                panic!("Clevis unbind idempotence test failed");
            }

            invariant(&backstore);

            if backstore.unbind_keyring().is_ok() {
                panic!("Keyring unbind check test failed");
            }

            invariant(&backstore);

            if !backstore.bind_clevis(
                "tang",
                &json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
            ).unwrap() {
                panic!(
                    "Clevis bind test failed"
                );
            }

            invariant(&backstore);

            if !backstore.unbind_keyring().unwrap() {
                panic!("Keyring unbind test failed");
            }

            invariant(&backstore);

            if backstore.unbind_keyring().unwrap() {
                panic!("Keyring unbind idempotence test failed");
            }

            invariant(&backstore);

            if backstore.unbind_clevis().is_ok() {
                panic!("Clevis unbind check test failed");
            }

            invariant(&backstore);

            if !backstore.bind_keyring(key_desc).unwrap() {
                panic!("Keyring bind test failed");
            }
        }

        crypt::insert_and_cleanup_key(paths, test_both);
    }

    #[test]
    fn clevis_real_test_both_initialize() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_clevis_both_initialize,
        );
    }

    #[test]
    #[should_panic]
    fn clevis_real_should_fail_test_both_initialize() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_clevis_both_initialize,
        );
    }

    #[test]
    fn clevis_loop_test_both_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 4, None),
            test_clevis_both_initialize,
        );
    }

    #[test]
    #[should_panic]
    fn clevis_loop_should_fail_test_both_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 4, None),
            test_clevis_both_initialize,
        );
    }
}

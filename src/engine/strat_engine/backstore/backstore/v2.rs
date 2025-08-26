// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle the backing store of a pool.

use std::{cmp, iter::once, path::PathBuf};

use chrono::{DateTime, Utc};
use either::Either;
use serde_json::Value;

use devicemapper::{
    CacheDev, CacheDevTargetTable, CacheTargetParams, DevId, Device, DmDevice, DmFlags, DmOptions,
    LinearDev, LinearDevTargetParams, LinearTargetParams, Sectors, TargetLine, TargetTable,
};

use crate::{
    engine::{
        strat_engine::{
            backstore::{
                backstore::InternalBackstore, blockdev::v2::StratBlockDev,
                blockdevmgr::BlockDevMgr, cache_tier::CacheTier, data_tier::DataTier,
                devices::UnownedDevices, shared::BlockSizeSummary,
            },
            crypt::{handle::v2::CryptHandle, manual_wipe, DEFAULT_CRYPT_DATA_OFFSET_V2},
            dm::{get_dm, list_of_backstore_devices, remove_optional_devices, DEVICEMAPPER_PATH},
            keys::{search_key_process, unset_key_process},
            metadata::MDADataSize,
            names::{format_backstore_ids, CacheRole},
            serde_structs::{BackstoreSave, CapSave, PoolFeatures, PoolSave, Recordable},
            writing::wipe_sectors,
        },
        types::{
            ActionAvailability, BlockDevTier, DevUuid, EncryptionInfo, InputEncryptionInfo,
            KeyDescription, OptionalTokenSlotInput, PoolUuid, SizedKeyMemory, TokenUnlockMethod,
            UnlockMechanism, ValidatedIntegritySpec, VolumeKeyKeyDescription,
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
    cap: Option<LinearDev>,
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
    if cap.is_some() {
        let dm = get_dm();
        dm.device_suspend(
            &DevId::Name(&dm_name),
            DmOptions::default().set_flags(DmFlags::DM_SUSPEND),
        )?;
        let table = CacheDevTargetTable::new(
            Sectors(0),
            origin.size(),
            CacheTargetParams::new(
                meta.device(),
                cache.device(),
                origin.device(),
                CACHE_BLOCK_SIZE,
                vec!["writethrough".into()],
                "default".to_owned(),
                Vec::new(),
            ),
        );
        dm.table_load(
            &DevId::Name(&dm_name),
            &table.to_raw_table(),
            DmOptions::default(),
        )?;
        dm.device_suspend(&DevId::Name(&dm_name), DmOptions::private())?;
    };
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

/// Set up the linear device on top of the data tier that can later be converted to a
/// cache device and serves as a placeholder for the device beneath encryption.
fn make_placeholder_dev(
    pool_uuid: PoolUuid,
    origin: &LinearDev,
) -> Result<LinearDev, StratisError> {
    let (dm_name, dm_uuid) = format_backstore_ids(pool_uuid, CacheRole::Cache);
    let target = vec![TargetLine::new(
        Sectors(0),
        origin.size(),
        LinearDevTargetParams::Linear(LinearTargetParams::new(origin.device(), Sectors(0))),
    )];
    LinearDev::setup(get_dm(), &dm_name, Some(&dm_uuid), target).map_err(StratisError::from)
}

/// This structure can allocate additional space to the upper layer, but it
/// cannot accept returned space. When it is extended to be able to accept
/// returned space the allocation algorithm will have to be revised.
#[derive(Debug)]
pub struct Backstore {
    /// Coordinate handling of blockdevs that back the cache. Optional, since
    /// this structure can operate without a cache.
    cache_tier: Option<CacheTier<StratBlockDev>>,
    /// Coordinates handling of the blockdevs that form the base.
    data_tier: DataTier<StratBlockDev>,
    /// A linear DM device.
    origin: Option<LinearDev>,
    /// A placeholder device to be converted to cache or a cache device.
    cache: Option<CacheDev>,
    /// A placeholder device to be converted to cache; necessary for reencryption support.
    placeholder: Option<LinearDev>,
    /// Either encryption information for a handle to be created at a later time or
    /// handle for encryption layer in backstore.
    enc: Option<Either<InputEncryptionInfo, CryptHandle>>,
    /// Data allocations on the cap device,
    allocs: Vec<(Sectors, Sectors)>,
    /// Metadata allocations on the cache or placeholder device.
    crypt_meta_allocs: Vec<(Sectors, Sectors)>,
}

impl InternalBackstore for Backstore {
    fn device(&self) -> Option<Device> {
        self.enc
            .as_ref()
            .and_then(|either| either.as_ref().right().map(|h| h.device()))
            .or_else(|| self.cache.as_ref().map(|c| c.device()))
            .or_else(|| self.placeholder.as_ref().map(|lin| lin.device()))
    }

    fn datatier_allocated_size(&self) -> Sectors {
        self.allocs.iter().map(|(_, length)| *length).sum()
    }

    fn datatier_usable_size(&self) -> Sectors {
        self.datatier_size() - self.datatier_metadata_size()
    }

    fn available_in_backstore(&self) -> Sectors {
        self.datatier_usable_size() - self.datatier_allocated_size()
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
            let next = self.calc_next_cap();
            let seg = (next, *size);
            chunks.push(seg);
            self.allocs.push(seg);
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
    /// Calculate next from all of the metadata and data allocations present in the backstore.
    fn calc_next_cache(&self) -> StratisResult<Sectors> {
        let mut all_allocs = if self.allocs.is_empty() {
            if matches!(self.enc, Some(Either::Right(_))) {
                return Err(StratisError::Msg(
                    "Metadata can only be allocated at the beginning of the cache device before the encryption device".to_string()
                ));
            } else {
                self.crypt_meta_allocs.clone()
            }
        } else {
            return Err(StratisError::Msg(
                "Metadata can only be allocated at the beginning of the cache device before the encryption device".to_string()
            ));
        };
        all_allocs.sort();

        for window in all_allocs.windows(2) {
            let (start, length) = (window[0].0, window[0].1);
            let start_next = window[1].0;
            assert_eq!(start + length, start_next);
        }

        Ok(all_allocs
            .last()
            .map(|(offset, len)| *offset + *len)
            .unwrap_or(Sectors(0)))
    }

    /// Calculate next from all of the metadata and data allocations present in the backstore.
    fn calc_next_cap(&self) -> Sectors {
        let mut all_allocs = if self.is_encrypted() {
            self.allocs.clone()
        } else {
            self.allocs
                .iter()
                .cloned()
                .chain(self.crypt_meta_allocs.iter().cloned())
                .collect::<Vec<_>>()
        };
        all_allocs.sort();

        for window in all_allocs.windows(2) {
            let (start, length) = (window[0].0, window[0].1);
            let start_next = window[1].0;
            assert_eq!(start + length, start_next);
        }

        all_allocs
            .last()
            .map(|(offset, len)| *offset + *len)
            .unwrap_or(Sectors(0))
    }

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
    /// self.origin.is_some() XOR self.cache.is_some()
    /// self.cache.is_some() <=> self.cache_tier.is_some()
    pub fn setup(
        pool_uuid: PoolUuid,
        pool_save: &PoolSave,
        datadevs: Vec<StratBlockDev>,
        cachedevs: Vec<StratBlockDev>,
        last_update_time: DateTime<Utc>,
        token_slot: TokenUnlockMethod,
        passphrase: Option<SizedKeyMemory>,
    ) -> StratisResult<Backstore> {
        let block_mgr = BlockDevMgr::new(datadevs, Some(last_update_time));
        let data_tier = match DataTier::setup(block_mgr, &pool_save.backstore.data_tier) {
            Ok(dt) => dt,
            Err(e) => {
                return Err(e);
            }
        };
        let (dm_name, dm_uuid) = format_backstore_ids(pool_uuid, CacheRole::OriginSub);
        let origin = match LinearDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            data_tier.segments.map_to_dm(),
        ) {
            Ok(origin) => origin,
            Err(e) => {
                return Err(StratisError::from(e));
            }
        };

        let (placeholder, cache, cache_tier, origin) = if !cachedevs.is_empty() {
            let block_mgr = BlockDevMgr::new(cachedevs, Some(last_update_time));
            match pool_save.backstore.cache_tier {
                Some(ref cache_tier_save) => {
                    let cache_tier = match CacheTier::setup(block_mgr, cache_tier_save) {
                        Ok(ct) => ct,
                        Err(e) => {
                            return Err(e);
                        }
                    };

                    let cache_device = match make_cache(pool_uuid, &cache_tier, origin, None, false)
                    {
                        Ok(cd) => cd,
                        Err(e) => {
                            return Err(e);
                        }
                    };
                    (None, Some(cache_device), Some(cache_tier), None)
                }
                None => {
                    let err_msg = "Cachedevs exist, but cache metadata does not exist";
                    return Err(StratisError::Msg(err_msg.into()));
                }
            }
        } else {
            let placeholder = make_placeholder_dev(pool_uuid, &origin)?;
            (Some(placeholder), None, None, Some(origin))
        };

        let metadata_enc_enabled = pool_save.features.contains(&PoolFeatures::Encryption);
        let crypt_physical_path = &once(DEVICEMAPPER_PATH)
            .chain(once(
                format_backstore_ids(pool_uuid, CacheRole::Cache)
                    .0
                    .to_string()
                    .as_str(),
            ))
            .collect::<PathBuf>();
        let has_header = match CryptHandle::load_metadata(crypt_physical_path, pool_uuid) {
            Ok(opt) => opt.is_some(),
            Err(e) => {
                return Err(e);
            }
        };
        let enc = match (metadata_enc_enabled, has_header, passphrase.as_ref()) {
            (true, true, pass) => {
                match CryptHandle::setup(crypt_physical_path, pool_uuid, token_slot, pass) {
                    Ok(opt) => {
                        if let Some(h) = opt {
                            Some(Either::Right(h))
                        } else {
                            return Err(StratisError::Msg(
                                "Metadata reported that encryption is enabled but no crypt header was found".to_string()
                            ));
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
            (true, _, _) => {
                return Err(StratisError::Msg(
                    "Metadata reported that encryption is enabled but no header was found"
                        .to_string(),
                ));
            }
            (false, true, _) => {
                return Err(StratisError::Msg(
                    "Metadata reported that encryption is disabled but header was found"
                        .to_string(),
                ));
            }
            (false, _, _) => None,
        };

        Ok(Backstore {
            data_tier,
            cache_tier,
            origin,
            cache,
            placeholder,
            enc,
            allocs: pool_save.backstore.cap.allocs.clone(),
            crypt_meta_allocs: pool_save.backstore.cap.crypt_meta_allocs.clone(),
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
        encryption_info: Option<&InputEncryptionInfo>,
        integrity_spec: ValidatedIntegritySpec,
    ) -> StratisResult<Backstore> {
        let data_tier = DataTier::<StratBlockDev>::new(
            BlockDevMgr::<StratBlockDev>::initialize(pool_uuid, devices, mda_data_size)?,
            integrity_spec,
        );

        let mut backstore = Backstore {
            data_tier,
            placeholder: None,
            cache_tier: None,
            cache: None,
            origin: None,
            enc: encryption_info.cloned().map(Either::Left),
            allocs: Vec::new(),
            crypt_meta_allocs: Vec::new(),
        };

        let size = DEFAULT_CRYPT_DATA_OFFSET_V2;
        if !backstore.meta_alloc_cache(&[size])? {
            return Err(StratisError::Msg(format!(
                "Failed to satisfy request in backstore for {size}"
            )));
        }

        Ok(backstore)
    }

    fn meta_alloc_cache(&mut self, sizes: &[Sectors]) -> StratisResult<bool> {
        let total_required = sizes.iter().cloned().sum();
        let available = self.available_in_backstore();
        if available < total_required {
            return Ok(false);
        }

        if !self.data_tier.alloc(sizes) {
            return Ok(false);
        }

        let mut chunks = Vec::new();
        for size in sizes {
            let next = self.calc_next_cache()?;
            let seg = (next, *size);
            chunks.push(seg);
            self.crypt_meta_allocs.push(seg);
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

        Ok(true)
    }

    /// Initialize the cache tier and add cachedevs to the backstore.
    ///
    /// Returns all `DevUuid`s of devices that were added to the cache on initialization.
    ///
    /// Precondition: Must be invoked only after some space has been allocated
    /// from the backstore. This ensures that there is certainly a cap device.
    // Precondition: self.cache.is_none() && self.placeholder.is_some()
    // Postcondition: self.cache.is_some() && self.placeholder.is_none()
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
                let bdm = BlockDevMgr::<StratBlockDev>::initialize(
                    pool_uuid,
                    devices,
                    MDADataSize::default(),
                )?;

                let cache_tier = CacheTier::new(bdm)?;

                let origin = self.origin
                    .take()
                    .expect("some space has already been allocated from the backstore => (cache_tier.is_none() <=> self.origin.is_some())");
                let placeholder = self.placeholder
                    .take()
                    .expect("some space has already been allocated from the backstore => (cache_tier.is_none() <=> self.placeholder.is_some())");

                let cache = make_cache(pool_uuid, &cache_tier, origin, Some(placeholder), true)?;

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
    // Precondition: self.origin.is_none() && self.cache.is_some()
    // Precondition: self.cache_key_desc has the desired key description
    // Precondition: self.cache.is_some() && self.origin.is_none()
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
        pool_uuid: PoolUuid,
        devices: UnownedDevices,
    ) -> StratisResult<Vec<DevUuid>> {
        self.data_tier.add(pool_uuid, devices)
    }

    /// Extend the cap device whether it is a cache or not. Create the DM
    /// device if it does not already exist. Return an error if DM
    /// operations fail. Use all segments currently allocated in the data tier.
    fn extend_cap_device(&mut self, pool_uuid: PoolUuid) -> StratisResult<()> {
        let create = match (
            self.cache.as_mut(),
            self.placeholder
                .as_mut()
                .and_then(|p| self.origin.as_mut().map(|o| (p, o))),
            self.enc.as_mut(),
        ) {
            (None, None, None) => true,
            (_, _, Some(Either::Left(_))) => true,
            (Some(cache), None, Some(Either::Right(handle))) => {
                let table = self.data_tier.segments.map_to_dm();
                cache.set_origin_table(get_dm(), table)?;
                cache.resume(get_dm())?;
                handle.resize(pool_uuid, None)?;
                false
            }
            (Some(cache), None, None) => {
                let table = self.data_tier.segments.map_to_dm();
                cache.set_origin_table(get_dm(), table)?;
                cache.resume(get_dm())?;
                false
            }
            (None, Some((placeholder, origin)), Some(Either::Right(handle))) => {
                let table = self.data_tier.segments.map_to_dm();
                origin.set_table(get_dm(), table)?;
                origin.resume(get_dm())?;
                let table = vec![TargetLine::new(
                    Sectors(0),
                    origin.size(),
                    LinearDevTargetParams::Linear(LinearTargetParams::new(
                        origin.device(),
                        Sectors(0),
                    )),
                )];
                placeholder.set_table(get_dm(), table)?;
                placeholder.resume(get_dm())?;
                handle.resize(pool_uuid, None)?;
                false
            }
            (None, Some((cap, linear)), None) => {
                let table = self.data_tier.segments.map_to_dm();
                linear.set_table(get_dm(), table)?;
                linear.resume(get_dm())?;
                let table = vec![TargetLine::new(
                    Sectors(0),
                    linear.size(),
                    LinearDevTargetParams::Linear(LinearTargetParams::new(
                        linear.device(),
                        Sectors(0),
                    )),
                )];
                cap.set_table(get_dm(), table)?;
                cap.resume(get_dm())?;
                false
            }
            _ => panic!("NOT (self.cache().is_some() AND self.origin.is_some())"),
        };

        if create {
            let table = self.data_tier.segments.map_to_dm();
            let (dm_name, dm_uuid) = format_backstore_ids(pool_uuid, CacheRole::OriginSub);
            let origin = LinearDev::setup(get_dm(), &dm_name, Some(&dm_uuid), table)?;
            let placeholder = make_placeholder_dev(pool_uuid, &origin)?;
            let handle = match self.enc {
                Some(Either::Left(ref einfo)) => Some(CryptHandle::initialize(
                    &once(DEVICEMAPPER_PATH)
                        .chain(once(
                            format_backstore_ids(pool_uuid, CacheRole::Cache)
                                .0
                                .to_string()
                                .as_str(),
                        ))
                        .collect::<PathBuf>(),
                    pool_uuid,
                    einfo,
                    None,
                )?),
                Some(Either::Right(_)) => unreachable!("Checked above"),
                None => {
                    manual_wipe(
                        &placeholder.devnode(),
                        Sectors(0),
                        DEFAULT_CRYPT_DATA_OFFSET_V2,
                    )?;
                    None
                }
            };
            self.origin = Some(origin);
            self.placeholder = Some(placeholder);
            self.enc = handle.map(Either::Right);
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
    /// of.
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

    /// Return a mutable reference to all the blockdevs that this pool has ownership
    /// of.
    pub fn blockdevs_mut(&mut self) -> Vec<(DevUuid, BlockDevTier, &mut StratBlockDev)> {
        match self.cache_tier {
            Some(ref mut cache) => self
                .data_tier
                .blockdevs_mut()
                .into_iter()
                .map(|(uuid, dev)| (uuid, BlockDevTier::Data, dev))
                .chain(
                    cache
                        .blockdevs_mut()
                        .into_iter()
                        .map(|(uuid, dev)| (uuid, BlockDevTier::Cache, dev)),
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
        self.enc
            .as_ref()
            .and_then(|either| either.as_ref().right().map(|handle| handle.size()))
            .or_else(|| self.placeholder.as_ref().map(|d| d.size()))
            .or_else(|| self.cache.as_ref().map(|d| d.size()))
            .unwrap_or(Sectors(0))
    }

    /// Destroy the entire store.
    pub fn destroy(&mut self, pool_uuid: PoolUuid) -> StratisResult<()> {
        if let Some(h) = self.enc.as_mut().and_then(|either| either.as_ref().right()) {
            h.wipe()?;
        }
        let devs = list_of_backstore_devices(pool_uuid);
        remove_optional_devices(devs)?;
        if let Some(ref mut cache_tier) = self.cache_tier {
            cache_tier.destroy()?;
        }
        self.data_tier.destroy()?;
        unset_key_process(&VolumeKeyKeyDescription::new(pool_uuid)).map(|_| ())
    }

    /// Teardown the DM devices in the backstore.
    pub fn teardown(&mut self, pool_uuid: PoolUuid) -> StratisResult<()> {
        let devs = list_of_backstore_devices(pool_uuid);
        remove_optional_devices(devs)?;
        if let Some(ref mut cache_tier) = self.cache_tier {
            cache_tier.block_mgr.teardown()?;
        }
        self.data_tier.block_mgr.teardown()?;
        unset_key_process(&VolumeKeyKeyDescription::new(pool_uuid)).map(|_| ())
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

    /// The space for the crypt meta device is allocated for the data tier,
    /// but not directly from the devices of the data tier. Although it is,
    /// strictly speaking, metadata, it is not included in the
    /// data_tier.metadata_size() result, which only includes metadata
    /// allocated directly from the blockdevs in the data tier.
    /// The space is included in the data_tier.allocated() result, since it is
    /// allocated from the assembled devices of the data tier.
    fn datatier_crypt_meta_size(&self) -> Sectors {
        self.crypt_meta_allocs.iter().map(|(_, len)| *len).sum()
    }

    /// Metadata size on the data tier, including crypt metadata space.
    pub fn datatier_metadata_size(&self) -> Sectors {
        self.datatier_crypt_meta_size() + self.data_tier.metadata_size()
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
        self.enc.is_some()
    }

    pub fn has_cache(&self) -> bool {
        self.cache_tier.is_some()
    }

    /// Get the encryption information for the backstore.
    pub fn encryption_info(&self) -> Option<&EncryptionInfo> {
        self.enc
            .as_ref()
            .and_then(|either| either.as_ref().right().map(|h| h.encryption_info()))
    }

    /// Bind device in the given backstore using the given clevis
    /// configuration.
    ///
    /// * Returns Ok(true) if the binding was performed.
    /// * Returns Ok(false) if the binding had already been previously performed and
    ///   nothing was changed.
    /// * Returns Err(_) if binding failed.
    pub fn bind_clevis(
        &mut self,
        token_slot: OptionalTokenSlotInput,
        pin: &str,
        clevis_info: &Value,
    ) -> StratisResult<Option<u32>> {
        let handle = self
            .enc
            .as_mut()
            .ok_or_else(|| StratisError::Msg("Pool is not encrypted".to_string()))?
            .as_mut()
            .right()
            .ok_or_else(|| {
                StratisError::Msg("No space has been allocated from the backstore".to_string())
            })?;

        match token_slot {
            OptionalTokenSlotInput::Legacy => {
                let ci = handle.encryption_info().single_clevis_info();
                if let Some((_, (existing_pin, existing_info))) = ci {
                    if existing_pin.as_str() == pin {
                        Ok(None)
                    } else {
                        Err(StratisError::Msg(format!(
                            "Crypt device has already been bound with pin {existing_pin} and config {existing_info}; \
                                requested pin {pin} and config {clevis_info} can't be applied"
                        )))
                    }
                } else {
                    handle.bind_clevis(None, pin, clevis_info).map(Some)
                }
            }
            OptionalTokenSlotInput::Some(k) => {
                // Ignore thumbprint if stratis:tang:trust_url is set in the clevis_info
                // config.
                let ci = handle.encryption_info().get_info(k);
                if let Some(UnlockMechanism::ClevisInfo((existing_pin, existing_info))) = ci {
                    if existing_pin == pin {
                        Ok(None)
                    } else {
                        Err(StratisError::Msg(format!(
                            "Crypt device has already been bound with pin {existing_pin} and config {existing_info}; \
                                requested pin {pin} and config {clevis_info} can't be applied"
                        )))
                    }
                } else {
                    handle.bind_clevis(Some(k), pin, clevis_info).map(Some)
                }
            }
            OptionalTokenSlotInput::None => {
                // Because idemptotence is checked based on pin, we can't reliably check whether
                // the binding has already been applied when no token slot is specified. As a
                // result, we default to adding the new config unless a token slot is specified.
                handle.bind_clevis(None, pin, clevis_info).map(Some)
            }
        }
    }

    /// Remove the keyring unlock mechanism specified by the token slot for the backstore.
    ///
    /// * Returns Ok(true) if the unbinding was performed.
    /// * Returns Ok(false) if the unbinding had already been previously performed and
    ///   nothing was changed.
    /// * Returns Err(_) if unbinding failed.
    pub fn unbind_keyring(&mut self, token_slot: Option<u32>) -> StratisResult<bool> {
        let handle = self
            .enc
            .as_mut()
            .ok_or_else(|| StratisError::Msg("Pool is not encrypted".to_string()))?
            .as_mut()
            .right()
            .ok_or_else(|| {
                StratisError::Msg("No space has been allocated from the backstore".to_string())
            })?;

        let ei = handle.encryption_info();
        match token_slot {
            Some(t) => {
                let info = ei.get_info(t);
                if let Some(UnlockMechanism::KeyDesc(_)) = info {
                    handle.unbind_keyring(t)?;
                    Ok(true)
                } else if let Some(UnlockMechanism::ClevisInfo(_)) = info {
                    Err(StratisError::Msg(format!("Token slot {t} could not be unbound from keyring; it is bound to a Clevis token")))
                } else {
                    Ok(false)
                }
            }
            None => {
                if let Some((t, _)) = ei.single_key_description() {
                    handle.unbind_keyring(t)?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    /// Remove the Clevis unlock mechanism specified by the token slot for the backstore.
    ///
    /// * Returns Ok(true) if the unbinding was performed.
    /// * Returns Ok(false) if the unbinding had already been previously performed and
    ///   nothing was changed.
    /// * Returns Err(_) if unbinding failed.
    pub fn unbind_clevis(&mut self, token_slot: Option<u32>) -> StratisResult<bool> {
        let handle = self
            .enc
            .as_mut()
            .ok_or_else(|| StratisError::Msg("Pool is not encrypted".to_string()))?
            .as_mut()
            .right()
            .ok_or_else(|| {
                StratisError::Msg("No space has been allocated from the backstore".to_string())
            })?;

        let ei = handle.encryption_info();
        match token_slot {
            Some(t) => {
                let info = ei.get_info(t);
                if let Some(UnlockMechanism::ClevisInfo(_)) = info {
                    handle.unbind_clevis(t)?;
                    Ok(true)
                } else if let Some(UnlockMechanism::KeyDesc(_)) = info {
                    Err(StratisError::Msg(format!("Token slot {t} could not be unbound from Clevis; it is bound to a key description token")))
                } else {
                    Ok(false)
                }
            }
            None => {
                let opt = ei.single_clevis_info();
                match opt {
                    Some((t, _)) => {
                        handle.unbind_clevis(t)?;
                        Ok(true)
                    }
                    None => Ok(false),
                }
            }
        }
    }

    /// Bind device in the given backstore to a passphrase using the
    /// given key description.
    ///
    /// * Returns Ok(true) if the binding was performed.
    /// * Returns Ok(false) if the binding had already been previously performed and
    ///   nothing was changed.
    /// * Returns Err(_) if binding failed.
    pub fn bind_keyring(
        &mut self,
        token_slot: OptionalTokenSlotInput,
        key_desc: &KeyDescription,
    ) -> StratisResult<Option<u32>> {
        let handle = self
            .enc
            .as_mut()
            .ok_or_else(|| StratisError::Msg("Pool is not encrypted".to_string()))?
            .as_mut()
            .right()
            .ok_or_else(|| {
                StratisError::Msg("No space has been allocated from the backstore".to_string())
            })?;

        match token_slot {
            OptionalTokenSlotInput::Legacy => {
                let info = handle.encryption_info().single_key_description();
                if let Some((_, kd)) = info {
                    if kd == key_desc {
                        Ok(None)
                    } else {
                        Err(StratisError::Msg(format!(
                            "Crypt device has already been bound with key description {}; \
                                requested key description {} can't be applied",
                            kd.as_application_str(),
                            key_desc.as_application_str(),
                        )))
                    }
                } else {
                    handle.bind_keyring(None, key_desc).map(Some)
                }
            }
            OptionalTokenSlotInput::Some(k) => {
                // Ignore thumbprint if stratis:tang:trust_url is set in the clevis_info
                // config.
                let info = handle.encryption_info().get_info(k);
                if let Some(UnlockMechanism::KeyDesc(ref kd)) = info {
                    if kd == key_desc {
                        Ok(None)
                    } else {
                        Err(StratisError::Msg(format!(
                            "Crypt device has already been bound with key description {}; \
                                requested key description {} can't be applied",
                            kd.as_application_str(),
                            key_desc.as_application_str(),
                        )))
                    }
                } else {
                    handle.bind_keyring(Some(k), key_desc).map(Some)
                }
            }
            OptionalTokenSlotInput::None => {
                // Ignore thumbprint if stratis:tang:trust_url is set in the clevis_info
                // config.
                let existing_config = handle
                    .encryption_info()
                    .all_key_descriptions()
                    .find(|(_, kd)| *kd == key_desc);
                if existing_config.is_some() {
                    Ok(None)
                } else {
                    handle.bind_keyring(None, key_desc).map(Some)
                }
            }
        }
    }

    /// Change the keyring passphrase associated with device in this pool.
    ///
    /// Returns:
    /// * Ok(None) if the pool is not currently bound to a keyring passphrase.
    /// * Ok(Some(true)) if the pool was successfully bound to the new key description.
    /// * Ok(Some(false)) if the pool is already bound to this key description.
    /// * Err(_) if an operation fails while changing the passphrase.
    pub fn rebind_keyring(
        &mut self,
        token_slot: Option<u32>,
        key_desc: &KeyDescription,
    ) -> StratisResult<Option<bool>> {
        let handle = self
            .enc
            .as_mut()
            .ok_or_else(|| StratisError::Msg("Pool is not encrypted".to_string()))?
            .as_mut()
            .right()
            .ok_or_else(|| {
                StratisError::Msg("No space has been allocated from the backstore".to_string())
            })?;

        let ei = handle.encryption_info();
        match token_slot {
            Some(t) => {
                let info = ei.get_info(t);
                match info {
                    Some(UnlockMechanism::KeyDesc(ref kd)) => if kd == key_desc {
                        Ok(Some(false))
                    } else {
                        handle.rebind_keyring(t, key_desc)?;
                        Ok(Some(true))
                    },
                    Some(UnlockMechanism::ClevisInfo(_)) => Err(StratisError::Msg(format!("Cannot rebind keyring implementation; token slot {t} is already bound to Clevis"))),
                    None => Ok(None)
                }
            }
            None => match ei.single_key_description() {
                Some((slot, kd)) => {
                    if kd == key_desc {
                        Ok(Some(false))
                    } else {
                        handle.rebind_keyring(slot, key_desc)?;
                        Ok(Some(true))
                    }
                }
                None => Ok(None),
            },
        }
    }

    /// Regenerate the Clevis bindings with the block devices in this pool using
    /// the same configuration.
    ///
    /// This method returns StratisResult<()> because the Clevis regen command
    /// will always change the metadata when successful. The command is not idempotent
    /// so this method will either fail to regenerate the bindings or it will
    /// result in a metadata change.
    pub fn rebind_clevis(&mut self, token_slot: Option<u32>) -> StratisResult<()> {
        let handle = self
            .enc
            .as_mut()
            .ok_or_else(|| StratisError::Msg("Pool is not encrypted".to_string()))?
            .as_mut()
            .right()
            .ok_or_else(|| {
                StratisError::Msg("No space has been allocated from the backstore".to_string())
            })?;

        let ei = handle.encryption_info();
        match token_slot {
            Some(t) => {
                let info = ei.get_info(t);
                match info {
                    Some(UnlockMechanism::KeyDesc(_)) => Err(StratisError::Msg(format!("Cannot rebind Clevis implementation; token slot {t} is already bound to a key description"))),
                    Some(UnlockMechanism::ClevisInfo(_)) => handle.rebind_clevis(t),
                    None => Err(StratisError::Msg(format!("Cannot rebind clevis implementation; token slot {t} is unbound"))),
                }
            }
            None => match ei.single_clevis_info() {
                Some((t, _)) => handle.rebind_clevis(t),
                None => Err(StratisError::Msg(
                    "Cannot rebind clevis implementation; no Clevis tokens are present".to_string(),
                )),
            },
        }
    }

    pub fn grow(&mut self, dev: DevUuid) -> StratisResult<bool> {
        self.data_tier.grow(dev)
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
            warn!("Disabling pool changes for this pool: {err}");
            ActionAvailability::NoPoolChanges
        } else if let Some(Err(err)) = cache_tier_bs_summary.map(|ct| ct.validate()) {
            // NOTE: This condition should be impossible. Since the cache is
            // always expanded to include all its devices, and an attempt to add
            // more devices than the cache can use causes the devices to be
            // rejected, there should be no unused devices in a cache. If, for
            // some reason this condition fails, though, NoPoolChanges would
            // be the correct state to put the pool in.
            warn!("Disabling pool changes for this pool: {err}");
            ActionAvailability::NoPoolChanges
        } else {
            ActionAvailability::Full
        }
    }

    /// Check whether a volume key is in the kernel keyring for a crypt device in the backstore.
    pub fn volume_key_is_loaded(uuid: PoolUuid) -> StratisResult<bool> {
        Ok(search_key_process(&VolumeKeyKeyDescription::new(uuid))?.is_some())
    }

    /// Load volume key into the kernel keyring for a crypt device in the backstore.
    pub fn load_volume_key(uuid: PoolUuid) -> StratisResult<bool> {
        let crypt_physical_path = &once(DEVICEMAPPER_PATH)
            .chain(once(
                format_backstore_ids(uuid, CacheRole::Cache)
                    .0
                    .to_string()
                    .as_str(),
            ))
            .collect::<PathBuf>();
        if Self::volume_key_is_loaded(uuid)? {
            Ok(false)
        } else {
            CryptHandle::load_vk_to_keyring(crypt_physical_path, uuid)?;
            Ok(true)
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
                allocs: self.allocs.clone(),
                crypt_meta_allocs: self.crypt_meta_allocs.clone(),
            },
            data_tier: self.data_tier.record(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{env, fs::OpenOptions, path::Path};

    use devicemapper::{CacheDevStatus, DataBlocks, DmOptions, IEC};

    use crate::engine::{
        strat_engine::{
            backstore::devices::{ProcessedPathInfos, UnownedDevices},
            cmd,
            metadata::device_identifiers,
            ns::{unshare_mount_namespace, MemoryFilesystem},
            tests::{crypt, loopbacked, real},
        },
        types::ValidatedIntegritySpec,
    };

    use super::*;

    const INITIAL_BACKSTORE_ALLOCATION: Sectors = CACHE_BLOCK_SIZE;

    /// Assert some invariants of the backstore
    /// * backstore.cache_tier.is_some() <=> backstore.cache.is_some() &&
    ///   backstore.cache_tier.is_some() => backstore.origin.is_none()
    /// * backstore's data tier allocated is equal to the size of the cap device
    /// * backstore's next index is always less than the size of the cap
    ///   device
    fn invariant(backstore: &Backstore) {
        assert!(
            (backstore.cache_tier.is_none() && backstore.cache.is_none())
                || (backstore.cache_tier.is_some()
                    && backstore.cache.is_some()
                    && backstore.origin.is_none())
        );
        assert_eq!(
            backstore.data_tier.allocated(),
            match (&backstore.origin, &backstore.cache) {
                (None, None) => DEFAULT_CRYPT_DATA_OFFSET_V2,
                (&None, Some(cache)) => cache.size(),
                (Some(linear), &None) => linear.size(),
                _ => panic!("impossible; see first assertion"),
            }
        );
        assert!(backstore.datatier_allocated_size() <= backstore.size());
        assert_eq!(
            backstore.datatier_allocated_size() + backstore.datatier_crypt_meta_size(),
            backstore.data_tier.allocated()
        );

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

        let mut backstore = Backstore::initialize(
            pool_uuid,
            initdatadevs,
            MDADataSize::default(),
            None,
            ValidatedIntegritySpec::default(),
        )
        .unwrap();

        invariant(&backstore);

        // Allocate space from the backstore so that the cap device is made.
        backstore
            .alloc(pool_uuid, &[INITIAL_BACKSTORE_ALLOCATION])
            .unwrap()
            .unwrap();

        let cache_uuids = backstore.init_cache(pool_uuid, initcachedevs).unwrap();

        invariant(&backstore);

        assert_eq!(cache_uuids.len(), initcachepaths.len());
        assert_matches!(backstore.origin, None);

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

        let devices1 = get_devices(paths1).unwrap();
        let devices2 = get_devices(paths2).unwrap();

        let mut backstore = Backstore::initialize(
            pool_uuid,
            devices1,
            MDADataSize::default(),
            None,
            ValidatedIntegritySpec::default(),
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

        assert_eq!(backstore.device(), old_device);

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
        let _memfs = MemoryFilesystem::new().unwrap();
        let pool_uuid = PoolUuid::new_v4();
        let ei = InputEncryptionInfo::new(
            vec![],
            vec![
                (None, (
                    "tang".to_string(),
                    json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
                ))
            ]
        ).expect("Empty data structure");
        let mut backstore = Backstore::initialize(
            pool_uuid,
            get_devices(paths).unwrap(),
            MDADataSize::default(),
            ei.as_ref(),
            ValidatedIntegritySpec::default(),
        )
        .unwrap();
        backstore.alloc(pool_uuid, &[Sectors(512)]).unwrap();
        cmd::udev_settle().unwrap();

        assert_matches!(
            backstore.bind_clevis(
                OptionalTokenSlotInput::None,
                "tang",
                &json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true})
            ),
            Ok(Some(_))
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
            let ei = InputEncryptionInfo::new(
                vec![(None, key_desc.to_owned())],
                vec![(None, (
                    "tang".to_string(),
                    json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
                ))],
            ).expect("Empty data structure");
            let mut backstore = Backstore::initialize(
                pool_uuid,
                get_devices(paths).unwrap(),
                MDADataSize::default(),
                ei.as_ref(),
                ValidatedIntegritySpec::default(),
            )
            .unwrap();
            cmd::udev_settle().unwrap();

            // Allocate space from the backstore so that the cap device is made.
            backstore
                .alloc(pool_uuid, &[2u64 * DEFAULT_CRYPT_DATA_OFFSET_V2])
                .unwrap()
                .unwrap();

            let ei = backstore.encryption_info().unwrap();
            let (kd_slot, _) = ei
                .all_key_descriptions()
                .next()
                .map(|(slot, kd)| (*slot, kd.clone()))
                .expect("Set one key description");
            let (ci_slot, _) = ei
                .all_clevis_infos()
                .next()
                .map(|(slot, ci)| (*slot, ci.clone()))
                .expect("Set one Clevis info");

            if backstore.bind_clevis(
                OptionalTokenSlotInput::Some(ci_slot),
                "tang",
                &json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
            ).unwrap().is_some() {
                panic!(
                    "Clevis bind idempotence test failed"
                );
            }

            invariant(&backstore);

            if backstore
                .bind_keyring(OptionalTokenSlotInput::None, key_desc)
                .unwrap()
                .is_some()
            {
                panic!("Keyring bind idempotence test failed")
            }

            if backstore
                .bind_keyring(OptionalTokenSlotInput::Some(kd_slot), key_desc)
                .unwrap()
                .is_some()
            {
                panic!("Keyring bind idempotence test failed")
            }

            invariant(&backstore);

            if !backstore.unbind_clevis(Some(ci_slot)).unwrap() {
                panic!("Clevis unbind test failed");
            }

            invariant(&backstore);

            if backstore.unbind_clevis(Some(ci_slot)).unwrap() {
                panic!("Clevis unbind idempotence test failed");
            }

            invariant(&backstore);

            if backstore.unbind_keyring(Some(kd_slot)).is_ok() {
                panic!("Keyring unbind check test failed");
            }

            invariant(&backstore);

            if backstore.bind_clevis(
                OptionalTokenSlotInput::Some(10),
                "tang",
                &json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
            ).unwrap().is_none() {
                panic!(
                    "Clevis bind test failed"
                );
            }

            invariant(&backstore);

            if !backstore.unbind_keyring(Some(kd_slot)).unwrap() {
                panic!("Keyring unbind test failed");
            }

            invariant(&backstore);

            if backstore.unbind_keyring(Some(kd_slot)).unwrap() {
                panic!("Keyring unbind idempotence test failed");
            }

            invariant(&backstore);

            if backstore.unbind_clevis(Some(10)).is_ok() {
                panic!("Clevis unbind check test failed");
            }

            invariant(&backstore);

            if backstore
                .bind_keyring(OptionalTokenSlotInput::Some(11), key_desc)
                .unwrap()
                .is_none()
            {
                panic!("Keyring bind test failed");
            }

            if backstore
                .bind_keyring(OptionalTokenSlotInput::Some(12), key_desc)
                .unwrap()
                .is_none()
            {
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

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Management of devices which are known to stratisd but not in a pool.

use std::{
    collections::{HashMap, HashSet},
    fs::OpenOptions,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde_json::Value;

use devicemapper::{Device, Sectors};

use crate::{
    engine::{
        strat_engine::{
            backstore::{identify_block_device, BlockDevPath, StratBlockDev, BDA},
            device::blkdev_size,
            devlinks::setup_pool_devlinks,
            pool::StratPool,
            serde_structs::{BackstoreSave, BaseBlockDevSave, PoolSave},
        },
        structures::Table,
        types::{BlockDevTier, DevUuid, Name, PoolUuid},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

/// Augment the devnodes structure with the BDA read from the device.
///
/// Precondition: All devices represented by devnodes have been already
/// identified as having the given pool UUID and their associated device
/// UUID. Thus, it is an error if the BDA's information does not match that
/// already known. It is also an error if the BDA can not be read at all
/// as that BDA may belong to the device with the most recently written
/// metadata.
pub fn add_bdas(
    pool_uuid: PoolUuid,
    devnodes: &HashMap<Device, (DevUuid, PathBuf)>,
) -> StratisResult<HashMap<Device, (DevUuid, PathBuf, BDA)>> {
    let mut infos = HashMap::new();
    for (device, (device_uuid, devnode)) in devnodes.iter() {
        let bda = BDA::load(&mut OpenOptions::new()
                            .read(true)
                            .open(devnode)?)?
            .ok_or_else(||
                        StratisError::Error(format!(
                                "Failed to read BDA from device {} with devnode {} which has previously been identified as a Stratis device with pool UUID {} and device UUID {}",
                                device,
                                devnode.display(),
                                pool_uuid.to_simple_ref(),
                                device_uuid.to_simple_ref())))?;
        if bda.pool_uuid() != pool_uuid || bda.dev_uuid() != *device_uuid {
            return Err(StratisError::Error(format!(
                        "BDA identifiers (pool UUID: {}, device UUID: {}) for device {} with devnode {} do not agree with previously read identifiers (pool UUID: {}, device UUID: {})",
                        bda.pool_uuid().to_simple_ref(),
                        bda.dev_uuid().to_simple_ref(),
                        device,
                        devnode.display(),
                        pool_uuid.to_simple_ref(),
                        device_uuid.to_simple_ref()
                        )));
        };
        infos.insert(*device, (pool_uuid, devnode.to_owned(), bda));
    }
    Ok(infos)
}

/// Get the most recent metadata from a set of devices.
/// Returns None if no metadata found for this pool on any device. This can
/// happen if the pool was constructed but failed in the interval before the
/// metadata could be written.
/// Returns an error if there is a last update time, but no metadata could
/// be obtained from any of the devices.
pub fn get_metadata(
    devnodes: &HashMap<Device, (DevUuid, PathBuf, BDA)>,
) -> StratisResult<Option<(DateTime<Utc>, PoolSave)>> {
    // Most recent time should never be None if this was a properly
    // created pool; this allows for the method to be called in other
    // circumstances.
    let most_recent_time = {
        match devnodes
            .iter()
            .filter_map(|(_, (_, _, ref bda))| bda.last_update_time())
            .max()
        {
            Some(time) => time,
            None => return Ok(None),
        }
    };

    // Try to read from all available devnodes that could contain most
    // recent metadata. In the event of errors, continue to try until all are
    // exhausted.
    devnodes
        .iter()
        .filter_map(|(_, (_, devnode, ref bda))| {
            if bda.last_update_time() == Some(most_recent_time) {
                OpenOptions::new()
                    .read(true)
                    .open(devnode)
                    .ok()
                    .and_then(|mut f| bda.load_state(&mut f).unwrap_or(None))
                    .and_then(|data| serde_json::from_slice(&data).ok())
            } else {
                None
            }
        })
        .next()
        .ok_or_else(|| {
            StratisError::Engine(
                ErrorEnum::NotFound,
                "timestamp indicates data was written, but no data successfully read".into(),
            )
        })
        .map(|psave| Some((*most_recent_time, psave)))
}

/// Get all the blockdevs corresponding to this pool that can be obtained from
/// the given devices. Sort the blockdevs in the order in which they were
/// recorded in the metadata.
/// Returns an error if the blockdevs obtained do not match the metadata.
/// Returns a tuple, of which the first are the data devs, and the second
/// are the devs that support the cache tier.
/// Precondition: Every device in devnodes has already been determined to
/// belong to one pool; all BDAs agree on their pool UUID.
pub fn get_blockdevs(
    backstore_save: &BackstoreSave,
    devnodes: HashMap<Device, (DevUuid, PathBuf, BDA)>,
) -> StratisResult<(Vec<StratBlockDev>, Vec<StratBlockDev>)> {
    let recorded_data_map: HashMap<DevUuid, (usize, &BaseBlockDevSave)> = backstore_save
        .data_tier
        .blockdev
        .devs
        .iter()
        .enumerate()
        .map(|(i, bds)| (bds.uuid, (i, bds)))
        .collect();

    let recorded_cache_map: HashMap<DevUuid, (usize, &BaseBlockDevSave)> =
        match backstore_save.cache_tier {
            Some(ref cache_tier) => cache_tier
                .blockdev
                .devs
                .iter()
                .enumerate()
                .map(|(i, bds)| (bds.uuid, (i, bds)))
                .collect(),
            None => HashMap::new(),
        };

    let mut segment_table: HashMap<DevUuid, Vec<(Sectors, Sectors)>> = HashMap::new();
    for seg in &backstore_save.data_tier.blockdev.allocs[0] {
        segment_table
            .entry(seg.parent)
            .or_insert_with(Vec::default)
            .push((seg.start, seg.length))
    }

    if let Some(ref cache_tier) = backstore_save.cache_tier {
        for seg in cache_tier.blockdev.allocs.iter().flat_map(|i| i.iter()) {
            segment_table
                .entry(seg.parent)
                .or_insert_with(Vec::default)
                .push((seg.start, seg.length))
        }
    }

    // Construct a single StratBlockDev. Return the tier to which the
    // blockdev has been found to belong. Returns an error if the block
    // device has shrunk, no metadata can be found for the block device,
    // or it is impossible to set up the device because the recorded
    // allocation information is impossible.
    fn get_blockdev(
        device: Device,
        devnode: &Path,
        bda: BDA,
        data_map: &HashMap<DevUuid, (usize, &BaseBlockDevSave)>,
        cache_map: &HashMap<DevUuid, (usize, &BaseBlockDevSave)>,
        segment_table: &HashMap<DevUuid, Vec<(Sectors, Sectors)>>,
    ) -> StratisResult<(BlockDevTier, StratBlockDev)> {
        // Return an error if apparent size of Stratis block device appears to
        // have decreased since metadata was recorded or if size of block
        // device could not be obtained.
        blkdev_size(&OpenOptions::new().read(true).open(devnode)?).and_then(|actual_size| {
            let actual_size_sectors = actual_size.sectors();
            let recorded_size = bda.dev_size().sectors();
            if actual_size_sectors < recorded_size {
                let err_msg = format!(
                    "Stratis device with device number {}, devnode {}, pool UUID {} and device UUID {} had recorded size {}, but actual size is less at {}",
                    device,
                    devnode.display(),
                    bda.pool_uuid().to_simple_ref(),
                    bda.dev_uuid().to_simple_ref(),
                    recorded_size,
                    actual_size_sectors
                );
                Err(StratisError::Engine(ErrorEnum::Error, err_msg))
            } else {
                Ok(())
            }
        })?;

        let dev_uuid = bda.dev_uuid();

        // Locate the device in the metadata using its uuid. Return the device
        // metadata and whether it was a cache or a datadev.
        let (tier, &(_, bd_save)) = data_map
            .get(&dev_uuid)
            .map(|bd_save| (BlockDevTier::Data, bd_save))
            .or_else(|| {
                cache_map
                    .get(&dev_uuid)
                    .map(|bd_save| (BlockDevTier::Cache, bd_save))
            })
            .ok_or_else(|| {
                let err_msg = format!(
                        "Stratis device with device number {}, devnode {}, pool UUID {} and device UUID {} had no record in pool metadata",
                        device,
                        devnode.display(),
                        bda.pool_uuid().to_simple_ref(),
                        bda.dev_uuid().to_simple_ref()
                    );
                StratisError::Engine(ErrorEnum::NotFound, err_msg)
            })?;

        // This should always succeed since the actual size is at
        // least the recorded size, so all segments should be
        // available to be allocated. If this fails, the most likely
        // conclusion is metadata corruption.
        let segments = segment_table.get(&dev_uuid);
        Ok((
            tier,
            StratBlockDev::new(
                device,
                // FIXME: This block device could represent an encrypted or
                // an unencrypted device.
                BlockDevPath::Unencrypted(devnode.to_owned()),
                bda,
                segments.unwrap_or(&vec![]),
                bd_save.user_info.clone(),
                bd_save.hardware_info.clone(),
                None,
            )?,
        ))
    }

    let (mut datadevs, mut cachedevs): (Vec<StratBlockDev>, Vec<StratBlockDev>) = (vec![], vec![]);
    for (device, (_, devnode, bda)) in devnodes {
        get_blockdev(
            device,
            &devnode,
            bda,
            &recorded_data_map,
            &recorded_cache_map,
            &segment_table,
        )
        .map(|(tier, blockdev)| {
            match tier {
                BlockDevTier::Data => &mut datadevs,
                BlockDevTier::Cache => &mut cachedevs,
            }
            .push(blockdev)
        })?;
    }

    // Verify that devices located are congruent with the metadata recorded
    // and generally consistent with expectations. If all seems correct,
    // sort the devices according to their order in the metadata.
    fn check_and_sort_devs(
        mut devs: Vec<StratBlockDev>,
        dev_map: &HashMap<DevUuid, (usize, &BaseBlockDevSave)>,
    ) -> StratisResult<Vec<StratBlockDev>> {
        let mut uuids = HashSet::new();
        let mut duplicate_uuids = Vec::new();
        for dev in &devs {
            let dev_uuid = dev.uuid();
            if !uuids.insert(dev_uuid) {
                duplicate_uuids.push(dev_uuid);
            }
        }

        if !duplicate_uuids.is_empty() {
            let err_msg = format!(
                "The following list of Stratis UUIDs were each claimed by more than one Stratis device: {}",
                duplicate_uuids.iter().map(|u| u.to_simple_ref().to_string()).collect::<Vec<_>>().join(", ")
            );
            return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
        }

        let recorded_uuids: HashSet<_> = dev_map.keys().cloned().collect();
        if uuids != recorded_uuids {
            let err_msg = format!(
                "UUIDs of devices found ({}) did not correspond with UUIDs specified in the metadata for this group of devices ({})",
                uuids.iter().map(|u| u.to_simple_ref().to_string()).collect::<Vec<_>>().join(", "),
                recorded_uuids.iter().map(|u| u.to_simple_ref().to_string()).collect::<Vec<_>>().join(", "),
            );
            return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
        }

        // Sort the devices according to their original location in the
        // metadata. Use a faster unstable sort, because the order of
        // devs before the sort is arbitrary and does not need to be
        // preserved.
        devs.sort_unstable_by_key(|dev| dev_map[&dev.uuid()].0);
        Ok(devs)
    }

    let datadevs = check_and_sort_devs(datadevs, &recorded_data_map)?;
    let cachedevs = check_and_sort_devs(cachedevs, &recorded_cache_map)?;

    Ok((datadevs, cachedevs))
}

/// Devices which stratisd has discovered but which have not been assembled
/// into pools.
#[derive(Debug)]
pub struct LiminalDevices {
    errored_pool_devices: HashMap<PoolUuid, HashMap<Device, (DevUuid, PathBuf)>>,
}

impl LiminalDevices {
    pub fn new() -> LiminalDevices {
        LiminalDevices {
            errored_pool_devices: HashMap::new(),
        }
    }

    /// Given a set of devices, try to set up a pool. If the setup fails,
    /// insert the devices into errored_pool_devices. Otherwise, return the pool.
    /// If there is a name conflict between the set of devices in devices
    /// and some existing pool, return an error.
    ///
    /// Precondition: pools.get_by_uuid(pool_uuid).is_none() &&
    ///               self.errored_pool_devices.get(pool_uuid).is_none()
    pub fn try_setup_pool(
        &mut self,
        pools: &Table<StratPool>,
        pool_uuid: PoolUuid,
        devices: HashMap<Device, (DevUuid, PathBuf)>,
    ) -> Option<(Name, StratPool)> {
        assert!(pools.get_by_uuid(pool_uuid).is_none());
        assert!(self.errored_pool_devices.get(&pool_uuid).is_none());

        // Setup a pool from constituent devices in the context of some already
        // setup pools.
        // Return None if the pool's metadata was not found. This is a
        // legitimate non-error condition, which may result if only a subset
        // of the pool's devices are in the set of devices being used.
        // Return an error on all other errors. Note that any one of these
        // errors could represent a temporary condition, that could be changed
        // by finding another device. So it is reasonable to treat them all
        // as loggable at the warning level, but not at the error level.
        // Precondition: every device in devices has already been determined to belong
        // to the pool with pool_uuid.
        fn setup_pool(
            pools: &Table<StratPool>,
            pool_uuid: PoolUuid,
            devices: &HashMap<Device, (DevUuid, PathBuf)>,
        ) -> Result<Option<(Name, StratPool)>, String> {
            let infos = match add_bdas(pool_uuid, devices) {
                Err(err) => return Err(format!(
                        "There was an error encountered when reading the BDAs for the devices found for pool with UUID {}: {}",
                        pool_uuid.to_simple_ref(),
                        err)),
                Ok(infos) => infos,
            };
            let (timestamp, metadata) = match get_metadata(&infos) {
                Err(err) => return Err(format!(
                        "There was an error encountered when reading the metadata for the devices found for pool with UUID {}: {}",
                        pool_uuid.to_simple_ref(),
                        err)),
                Ok(None) => return Ok(None),
                Ok(Some((timestamp, metadata))) => (timestamp, metadata),
            };

            if let Some((uuid, _)) = pools.get_by_name(&metadata.name) {
                return Err(format!(
                        "There is a pool name conflict. The devices currently being processed have been identified as belonging to the pool with UUID {} and name {}, but a pool with the same name and UUID {} is already active",
                        pool_uuid.to_simple_ref(),
                        &metadata.name,
                        uuid.to_simple_ref()));
            }

            let (datadevs, cachedevs) = match get_blockdevs(&metadata.backstore, infos) {
                Err(err) => return Err(format!(
                        "There was an error encountered when calculating the block devices for pool with UUID {} and name {}: {}",
                        pool_uuid.to_simple_ref(),
                        &metadata.name,
                        err)),
                Ok((datadevs, cachedevs)) => (datadevs, cachedevs),
            };

            StratPool::setup(pool_uuid, datadevs, cachedevs, timestamp, &metadata)
                .map_err(|err| {
                    format!(
                        "An attempt to set up pool with UUID {} from the assembled devices failed: {}",
                        pool_uuid.to_simple_ref(),
                        err
                    )
                })
                .map(Some)
        }

        let result = setup_pool(pools, pool_uuid, &devices);

        if let Err(err) = &result {
            warn!("{}", err);
        }

        match result {
            Ok(Some((pool_name, pool))) => {
                setup_pool_devlinks(&pool_name, &pool);
                info!(
                    "Pool with name \"{}\" and UUID \"{}\" set up",
                    pool_name,
                    pool_uuid.to_simple_ref()
                );
                Some((pool_name, pool))
            }
            _ => {
                self.errored_pool_devices.insert(pool_uuid, devices);
                None
            }
        }
    }

    /// Given some information gathered about a single Stratis device, determine
    /// whether or not a pool can be constructed, and if it can, construct the
    /// pool and return the newly constructed pool. If the device appears to
    /// belong to a pool that has already been set up assume that no further
    /// processing is required and return None. If there is an error
    /// constructing the pool, retain the set of devices.
    pub fn block_evaluate(
        &mut self,
        pools: &Table<StratPool>,
        event: &libudev::Event,
    ) -> Option<(PoolUuid, Name, StratPool)> {
        identify_block_device(event.device()).and_then(move |info| {
            let pool_uuid = info.identifiers.pool_uuid;
            if pools.contains_uuid(pool_uuid) {
                // FIXME: There is the possibilty of an error condition here,
                // if the device found is not in the already set up pool.
                None
            } else {
                let mut devices = self
                    .errored_pool_devices
                    .remove(&pool_uuid)
                    .unwrap_or_else(HashMap::new);

                if devices
                    .insert(
                        info.device_number,
                        (info.identifiers.device_uuid, info.devnode),
                    )
                    .is_none()
                {
                    info!(
                        "Stratis block device with device number \"{}\", pool UUID \"{}\", and device UUID \"{}\" discovered, i.e., identified for the first time during this execution of stratisd",
                        info.device_number,
                        info.identifiers.pool_uuid.to_simple_ref(),
                        info.identifiers.device_uuid.to_simple_ref(),
                    );
                }

                // FIXME: An attempt to set up the pool is made, even if no
                // new device has been added to the set of devices that appear
                // to belong to the pool. The reason for this is that there
                // may be many causes of failure to set up a pool, and that
                // it may be worth another try. If an attempt to setup the
                // pool is only made on discovery of a new device that may
                // leave a pool that could be set up in limbo forever. An
                // alternative, where the user can explicitly ask to try to
                // set up an incomplete pool would be a better choice.
                self.try_setup_pool(pools, pool_uuid, devices).map(|(name, pool)| (pool_uuid, name, pool))
            }
        })
    }

    /// Generate a JSON report giving some information about the internals
    /// of these devices.
    pub fn report(&self) -> Value {
        Value::Array(self.errored_pool_devices.iter().map(|(uuid, map)| {
            json!({
                "pool_uuid": uuid.to_simple_ref().to_string(),
                "devices": Value::Array(map.values().map(|(_, p)| Value::from(p.display().to_string())).collect()),
            })
        }).collect())
    }
}

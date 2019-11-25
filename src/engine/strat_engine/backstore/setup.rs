// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle initial setup steps for a pool.
// Initial setup steps are steps that do not alter the environment.

use std::{
    collections::{HashMap, HashSet},
    fs::OpenOptions,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use libudev;
use serde_json;

use devicemapper::{devnode_to_devno, Device, Sectors};

use crate::{
    engine::{
        strat_engine::{
            backstore::{
                blockdev::StratBlockDev,
                device::DevOwnership,
                metadata::{device_identifiers, BDA},
                udev::{
                    block_enumerator, decide_ownership, is_multipath_member, stratis_enumerator,
                },
            },
            device::blkdev_size,
            serde_structs::{BackstoreSave, BaseBlockDevSave, PoolSave},
        },
        types::{BlockDevTier, DevUuid, PoolUuid},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

/// Retrieve all the block devices on the system that have a Stratis signature.
fn get_stratis_block_devices() -> StratisResult<Vec<PathBuf>> {
    let devices = {
        let context = libudev::Context::new()?;
        let mut enumerator = stratis_enumerator(&context)?;

        enumerator
            .scan_devices()?
            .filter(|dev| dev.is_initialized())
            .filter(|dev| !is_multipath_member(dev).unwrap_or(true))
            .filter_map(|i| i.devnode().map(|d| d.into()))
            .collect::<Vec<PathBuf>>()
    };

    if devices.is_empty() {
        // No Stratis devices have been found, possible reasons are:
        // 1. There are none
        // 2. There are some but libblkid version is less than 2.32, so
        // Stratis devices are not recognized by udev.
        // 3. There are many incomplete udev entries, because this code is
        // being run before the udev database is populated.
        //
        // Try again to find Stratis block devices, but this time enumerate
        // all block devices, not just all the ones that can be identified
        // as Stratis blockdevs by udev, and then scrutinize each one
        // using various methods.
        let context = libudev::Context::new()?;
        let mut enumerator = block_enumerator(&context)?;
        Ok(enumerator
            .scan_devices()?
            .filter(|dev| dev.is_initialized())
            .filter_map(|dev| {
                dev.devnode().and_then(|devnode| {
                    decide_ownership(&dev)
                        .and_then(|decision| DevOwnership::from_udev_ownership(&decision, devnode))
                        .ok()
                        .and_then(|ownership| ownership.stratis_identifiers())
                        .map(|_| devnode.into())
                })
            })
            .collect())
    } else {
        Ok(devices)
    }
}

/// Find all Stratis devices.
///
/// Returns a map of pool uuids to a map of devices to devnodes for each pool.
pub fn find_all() -> StratisResult<HashMap<PoolUuid, HashMap<Device, PathBuf>>> {
    let mut pool_map = HashMap::new();

    for devnode in get_stratis_block_devices()? {
        match devnode_to_devno(&devnode)? {
            None => continue,
            Some(devno) => {
                if let Some((pool_uuid, _)) =
                    device_identifiers(&mut OpenOptions::new().read(true).open(&devnode)?)?
                {
                    pool_map
                        .entry(pool_uuid)
                        .or_insert_with(HashMap::new)
                        .insert(Device::from(devno), devnode);
                }
            }
        }
    }
    Ok(pool_map)
}

/// Get the most recent metadata from a set of Devices for a given pool UUID.
/// Returns None if no metadata found for this pool.
#[allow(clippy::implicit_hasher)]
pub fn get_metadata(
    pool_uuid: PoolUuid,
    devnodes: &HashMap<Device, PathBuf>,
) -> StratisResult<Option<(DateTime<Utc>, PoolSave)>> {
    // Get pairs of device nodes and matching BDAs
    // If no BDA, or BDA UUID does not match pool UUID, skip.
    // If there is an error reading the BDA, error. There could have been
    // vital information on that BDA, for example, it may have contained
    // the newest metadata.
    let mut bdas = Vec::new();
    for devnode in devnodes.values() {
        let bda = BDA::load(&mut OpenOptions::new().read(true).open(devnode)?)?;
        if let Some(bda) = bda {
            if bda.pool_uuid() == pool_uuid {
                bdas.push((devnode, bda));
            }
        }
    }

    // Most recent time should never be None if this was a properly
    // created pool; this allows for the method to be called in other
    // circumstances.
    let most_recent_time = {
        match bdas
            .iter()
            .filter_map(|&(_, ref bda)| bda.last_update_time())
            .max()
        {
            Some(time) => time,
            None => return Ok(None),
        }
    };

    // Try to read from all available devnodes that could contain most
    // recent metadata. In the event of errors, continue to try until all are
    // exhausted.
    bdas.iter()
        .filter_map(|&(devnode, ref bda)| {
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
/// Returns an error if a BDA can not be read or can not be found on any
/// blockdev in devnodes.
/// Returns an error if the blockdevs obtained do not match the metadata.
/// Returns a tuple, of which the first are the data devs, and the second
/// are the devs that support the cache tier.
/// Precondition: Every device in devnodes has already been determined to
/// belong to the pool with the specified pool uuid.
#[allow(clippy::implicit_hasher)]
pub fn get_blockdevs(
    pool_uuid: PoolUuid,
    backstore_save: &BackstoreSave,
    devnodes: &HashMap<Device, PathBuf>,
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
                    "Stratis device with device number {}, devnode {}, pool UUID {} and device UUID {} had recorded size ({}), but actual size is less at ({})",
                    device,
                    devnode.display(),
                    bda.pool_uuid(),
                    bda.dev_uuid(),
                    recorded_size,
                    actual_size
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
                        bda.pool_uuid(),
                        bda.dev_uuid()
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
                devnode.to_owned(),
                bda,
                segments.unwrap_or(&vec![]),
                bd_save.user_info.clone(),
                bd_save.hardware_info.clone(),
            )?,
        ))
    }

    let (mut datadevs, mut cachedevs): (Vec<StratBlockDev>, Vec<StratBlockDev>) = (vec![], vec![]);
    for (device, devnode) in devnodes {
        let bda = BDA::load(&mut OpenOptions::new().read(true).open(devnode)?)?.ok_or_else(|| {
            StratisError::Engine(ErrorEnum::NotFound,
                                                 format!("Device {} with devnode {} was previously determined to belong to pool with uuid {} but no BDA was found",
                                                 device,
                                                 devnode.display(),
                                                 pool_uuid))
        })?;

        get_blockdev(
            *device,
            devnode,
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

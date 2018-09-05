// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle initial setup steps for a pool.
// Initial setup steps are steps that do not alter the environment.

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use libudev;
use serde_json;

use devicemapper::{Device, Sectors};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::types::{BlockDevTier, DevUuid, PoolUuid};
use super::super::super::udev::{get_device_devnode, get_udev};

use super::super::serde_structs::{BackstoreSave, BaseBlockDevSave, PoolSave};

use super::blockdev::StratBlockDev;
use super::device::blkdev_size;
use super::metadata::{device_identifiers, BDA};
use super::udev::{must_ignore, unclaimed};

/// Find all Stratis devices.
///
/// Returns a map of pool uuids to a map of devices to devnodes for each pool.
pub fn find_all() -> StratisResult<HashMap<PoolUuid, HashMap<Device, PathBuf>>> {
    let context = get_udev();

    let mut enumerator = libudev::Enumerator::new(context)?;
    enumerator.match_subsystem("block")?;
    enumerator.match_property("ID_FS_TYPE", "stratis")?;

    // Skip any block devices w/out a devnode and a device number.
    let devices: Vec<(Device, PathBuf)> = enumerator
        .scan_devices()?
        .filter(|x| !must_ignore(x))
        .filter_map(|x| get_device_devnode(&x))
        .collect();

    // TODO: If at some point it is guaranteed that libblkid version is
    // not less than that required to identify Stratis devices, this block
    // can be removed.
    let (devices, only_stratis) = if devices.is_empty() {
        // There are no Stratis devices or the only Stratis devices are
        // multipath members OR libblkid is an early version that
        // doesn't support identifying Stratis devices. Fall back to using
        // udev to get all devices that are lacking any signature which
        // identifies them as belonging to some other system or application.

        let mut enumerator = libudev::Enumerator::new(context)?;
        enumerator.match_subsystem("block")?;

        (
            enumerator
                .scan_devices()?
                .filter(|d| unclaimed(d) && !must_ignore(d))
                .filter_map(|x| get_device_devnode(&x))
                .collect(),
            false,
        )
    } else {
        (devices, true)
    };

    let mut pool_map = HashMap::new();
    if only_stratis {
        // If these are devices that udev has identified as Stratis but not
        // multipath members:
        // 1. Assume that not being able to open the device is an error.
        // 2. Return an error if the device has no Stratis header.
        for (device, devnode) in devices {
            match device_identifiers(&mut OpenOptions::new().read(true).open(&devnode)?)? {
                Some((pool_uuid, _)) => {
                    pool_map
                        .entry(pool_uuid)
                        .or_insert_with(HashMap::new)
                        .insert(device, devnode);
                }
                None => {
                    return Err(StratisError::Engine(ErrorEnum::Invalid,
                                                "udev has identified this device as a Stratis device, but no Stratis header was found on the device".into()));
                }
            }
        }
    } else {
        // If these are only unclaimed devices or devices that are "certainly"
        // not multipath member devices (a fallback when libblkid is
        // not a recent version that can identify Stratis devices):
        // 1. Assume that failure to open the device is not an error.
        // 2. Do not treat failure to find the Stratis header as an error.
        // 3. Do not treat failure when reading the device for the Stratis
        // header as an error.
        for (device, devnode) in devices {
            if let Ok(Some((pool_uuid, _))) = OpenOptions::new()
                .read(true)
                .open(&devnode)
                .map_err(|e| e.into())
                .and_then(|mut file| device_identifiers(&mut file))
            {
                pool_map
                    .entry(pool_uuid)
                    .or_insert_with(HashMap::new)
                    .insert(device, devnode);
            }
        }
    }

    Ok(pool_map)
}

/// Get the most recent metadata from a set of Devices for a given pool UUID.
/// Returns None if no metadata found for this pool.
#[allow(implicit_hasher)]
pub fn get_metadata(
    pool_uuid: PoolUuid,
    devnodes: &HashMap<Device, PathBuf>,
) -> StratisResult<Option<PoolSave>> {
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
        match bdas.iter()
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
    for &(devnode, ref bda) in bdas.iter()
        .filter(|&&(_, ref bda)| bda.last_update_time() == Some(most_recent_time))
    {
        let poolsave = OpenOptions::new()
            .read(true)
            .open(devnode)
            .ok()
            .and_then(|mut f| bda.load_state(&mut f).ok())
            .and_then(|opt| opt)
            .and_then(|data| serde_json::from_slice(&data).ok());

        if poolsave.is_some() {
            return Ok(poolsave);
        }
    }

    // If no data has yet returned, we have an error. That is, we should have
    // some metadata, because we have a most recent time, but we failed to
    // get any.
    let err_str = "timestamp indicates data was written, but no data successfully read";
    Err(StratisError::Engine(ErrorEnum::NotFound, err_str.into()))
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
#[allow(implicit_hasher)]
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
            let recorded_size = bda.dev_size();
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
        ).map(|(tier, blockdev)| {
            match tier {
                BlockDevTier::Data => &mut datadevs,
                BlockDevTier::Cache => &mut cachedevs,
            }.push(blockdev)
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
                "The following list of Stratis UUIDs were each claimed by more than one Stratis device: {:?}",
                duplicate_uuids
            );
            return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
        }

        let recorded_uuids: HashSet<_> = dev_map.keys().cloned().collect();
        if uuids != recorded_uuids {
            let err_msg = format!(
                "UUIDs of devices found ({:?}) did not correspond with UUIDs specified in the metadata for this group of devices ({:?})",
                uuids,
                recorded_uuids
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

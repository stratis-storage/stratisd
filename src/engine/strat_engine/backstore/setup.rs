// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle initial setup steps for a pool.
// Initial setup steps are steps that do not alter the environment.

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use serde_json;

use devicemapper::{devnode_to_devno, Device, Sectors};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::types::{BlockDevTier, DevUuid, PoolUuid};

use super::super::serde_structs::{BackstoreSave, BlockDevSave, PoolSave};

use super::blockdev::StratBlockDev;
use super::device::{blkdev_size, is_stratis_device};
use super::metadata::BDA;
use super::util::get_stratis_block_devices;

/// Find all Stratis devices.
///
/// Returns a map of pool uuids to a map of devices to devnodes for each pool.
pub fn find_all() -> StratisResult<HashMap<PoolUuid, HashMap<Device, PathBuf>>> {
    let mut pool_map = HashMap::new();

    for devnode in get_stratis_block_devices()? {
        match devnode_to_devno(&devnode)? {
            None => continue,
            Some(devno) => {
                is_stratis_device(&devnode)?.and_then(|pool_uuid| {
                    pool_map
                        .entry(pool_uuid)
                        .or_insert_with(HashMap::new)
                        .insert(Device::from(devno), devnode)
                });
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
/// the given devices.
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
    let recorded_data_map: HashMap<DevUuid, (usize, &BlockDevSave)> = backstore_save
        .data_devs
        .iter()
        .enumerate()
        .map(|(i, bds)| (bds.uuid, (i, bds)))
        .collect();

    let recorded_cache_map: HashMap<DevUuid, (usize, &BlockDevSave)> =
        match backstore_save.cache_devs {
            Some(ref cache_devs) => cache_devs
                .iter()
                .enumerate()
                .map(|(i, bds)| (bds.uuid, (i, bds)))
                .collect(),
            None => HashMap::new(),
        };

    let mut segment_table: HashMap<DevUuid, Vec<(Sectors, Sectors)>> = HashMap::new();
    for seg in &backstore_save.data_segments {
        segment_table
            .entry(seg.0)
            .or_insert_with(Vec::default)
            .push((seg.1, seg.2))
    }
    if let Some(ref segs) = backstore_save.cache_segments {
        for seg in segs {
            segment_table
                .entry(seg.0)
                .or_insert_with(Vec::default)
                .push((seg.1, seg.2))
        }
    }
    if let Some(ref segs) = backstore_save.meta_segments {
        for seg in segs {
            segment_table
                .entry(seg.0)
                .or_insert_with(Vec::default)
                .push((seg.1, seg.2))
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
        data_map: &HashMap<DevUuid, (usize, &BlockDevSave)>,
        cache_map: &HashMap<DevUuid, (usize, &BlockDevSave)>,
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
        let (tier, (_, bd_save)) = data_map
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

    let (mut datadevs, mut cachedevs) = (vec![], vec![]);
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

    // Verify that datadevs found match datadevs recorded.
    let current_data_uuids: HashSet<_> = datadevs.iter().map(|b| b.uuid()).collect();
    let recorded_data_uuids: HashSet<_> = recorded_data_map.keys().cloned().collect();
    if current_data_uuids != recorded_data_uuids {
        let err_msg = "Recorded data dev UUIDs != discovered datadev UUIDs";
        return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg.into()));
    }

    if datadevs.len() != current_data_uuids.len() {
        let err_msg = "Duplicate data devices found in environment";
        return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg.into()));
    }

    // Verify that cachedevs found match cachedevs recorded.
    let current_cache_uuids: HashSet<_> = cachedevs.iter().map(|b| b.uuid()).collect();
    let recorded_cache_uuids: HashSet<_> = recorded_cache_map.keys().cloned().collect();
    if current_cache_uuids != recorded_cache_uuids {
        let err_msg = "Recorded cache dev UUIDs != discovered cachedev UUIDs";
        return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg.into()));
    }

    if cachedevs.len() != current_cache_uuids.len() {
        let err_msg = "Duplicate cache devices found in environment";
        return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg.into()));
    }

    Ok((datadevs, cachedevs))
}

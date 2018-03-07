// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle initial setup steps for a pool.
// Initial setup steps are steps that do not alter the environment.

use std::collections::{HashMap, HashSet};
use std::fs::{OpenOptions, read_dir};
use std::io::ErrorKind;
use std::ops::Sub;
use std::path::PathBuf;

use nix::errno::Errno;
use serde_json;

use devicemapper::{Device, Sectors, devnode_to_devno};

use super::super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::super::types::{DevUuid, PoolUuid};

use super::super::engine::DevOwnership;
use super::super::serde_structs::{BackstoreSave, BlockDevSave, PoolSave};

use super::blockdev::StratBlockDev;
use super::device::blkdev_size;
use super::metadata::{BDA, StaticHeader};


/// Determine if devnode is a stratis device, if it is we will add  pool uuid and device
/// information to pool_map and return pool uuid.
pub fn is_stratis_device(devnode: &PathBuf) -> EngineResult<Option<PoolUuid>> {
    match OpenOptions::new().read(true).open(&devnode) {
        Ok(mut f) => {
            if let DevOwnership::Ours(pool_uuid, _) = StaticHeader::determine_ownership(&mut f)? {
                Ok(Some(pool_uuid))
            } else {
                Ok(None)
            }
        }
        Err(err) => {
            // There are some reasons for OpenOptions::open() to return an error
            // which are not reasons for this method to return an error.
            // Try to distinguish. Non-error conditions are:
            //
            // 1. ENXIO: The device does not exist anymore. This means that the device
            // was volatile for some reason; in that case it can not belong to
            // Stratis so it is safe to ignore it.
            //
            // 2. ENOMEDIUM: The device has no medium. An example of this case is an
            // empty optical drive.
            //
            // Note that it is better to be conservative and return with an
            // error in any case where failure to read the device could result
            // in bad data for Stratis. Additional exceptions may be added,
            // but only with a complete justification.
            match err.kind() {
                ErrorKind::NotFound => Ok(None),
                _ => {
                    if let Some(errno) = err.raw_os_error() {
                        match Errno::from_i32(errno) {
                            Errno::ENXIO | Errno::ENOMEDIUM => Ok(None),
                            _ => Err(EngineError::Io(err)),
                        }
                    } else {
                        Err(EngineError::Io(err))
                    }
                }
            }
        }
    }
}

/// Find all Stratis devices.
///
/// Returns a map of pool uuids to a map of devices to devnodes for each pool.
pub fn find_all() -> EngineResult<HashMap<PoolUuid, HashMap<Device, PathBuf>>> {

    let mut pool_map = HashMap::new();
    let mut devno_set = HashSet::new();
    for dir_e in read_dir("/dev")? {
        let dir_e = dir_e?;
        let devnode = dir_e.path();

        match devnode_to_devno(&devnode)? {
            None => continue,
            Some(devno) => {
                if devno_set.insert(devno) {
                    is_stratis_device(&devnode)?
                        .and_then(|pool_uuid| {
                                      pool_map
                                          .entry(pool_uuid)
                                          .or_insert_with(HashMap::new)
                                          .insert(Device::from(devno), devnode)
                                  });
                } else {
                    continue;
                }
            }
        }
    }

    Ok(pool_map)
}

/// Get the most recent metadata from a set of Devices for a given pool UUID.
/// Returns None if no metadata found for this pool.
#[allow(implicit_hasher)]
pub fn get_metadata(pool_uuid: PoolUuid,
                    devnodes: &HashMap<Device, PathBuf>)
                    -> EngineResult<Option<PoolSave>> {

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
                  .max() {
            Some(time) => time,
            None => return Ok(None),
        }
    };

    // Try to read from all available devnodes that could contain most
    // recent metadata. In the event of errors, continue to try until all are
    // exhausted.
    for &(devnode, ref bda) in
        bdas.iter()
            .filter(|&&(_, ref bda)| bda.last_update_time() == Some(most_recent_time)) {

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
    Err(EngineError::Engine(ErrorEnum::NotFound, err_str.into()))
}

/// Synthesize information obtained from the metadata and records of the
/// actual devnodes left on the device into a list of StratBlockDevs
/// sorted according to their order in the metadata.
/// Returns an error if a dev's UUID can not be found in the metadata.
fn get_blockdevs_internal(pool_uuid: PoolUuid,
                          blockdev_map: &HashMap<DevUuid, (usize, &BlockDevSave)>,
                          segment_table: &HashMap<DevUuid, Vec<(Sectors, Sectors)>>,
                          devnodes: &HashMap<Device, PathBuf>)
                          -> EngineResult<Vec<StratBlockDev>> {
    let mut blockdevs = vec![];
    for (device, devnode) in devnodes {
        let bda = BDA::load(&mut OpenOptions::new().read(true).open(devnode)?)?;
        if let Some(bda) = bda {
            if bda.pool_uuid() == pool_uuid {
                let actual_size = blkdev_size(&OpenOptions::new().read(true).open(devnode)?)?
                    .sectors();

                if actual_size < bda.dev_size() {
                    let err_msg = format!("actual blockdev size ({}) < recorded size ({})",
                                          actual_size,
                                          bda.dev_size());

                    return Err(EngineError::Engine(ErrorEnum::Error, err_msg));
                }

                let dev_uuid = bda.dev_uuid();
                let &(index, bd_save) = blockdev_map
                    .get(&dev_uuid)
                    .ok_or_else(|| {
                                    let err_msg = format!("Blockdev {} not found in metadata",
                                                          dev_uuid);
                                    EngineError::Engine(ErrorEnum::NotFound, err_msg)
                                })?;

                // This should always succeed since the actual size is at
                // least the recorded size, so all segments should be
                // available to be allocated. If this fails, the most likely
                // conclusion is metadata corruption.
                let segments = segment_table.get(&bda.dev_uuid());
                blockdevs.push((index,
                                StratBlockDev::new(*device,
                                                   devnode.to_owned(),
                                                   bda,
                                                   segments.unwrap_or(&vec![]),
                                                   bd_save.user_info.clone(),
                                                   bd_save.hardware_info.clone())?));
            }
        }
    }
    blockdevs.sort_by_key(|&(index, _)| index);
    Ok(blockdevs
           .into_iter()
           .map(|(_, bd)| bd)
           .collect::<Vec<_>>())
}

/// Get all the blockdevs corresponding to this pool that can be obtained from
/// the given devices. Return them in the order in which they were written
/// in the metadata.
/// Returns an error if the blockdevs obtained do not match the metadata.
#[allow(implicit_hasher)]
pub fn get_blockdevs(pool_uuid: PoolUuid,
                     backstore_save: &BackstoreSave,
                     devnodes: &HashMap<Device, PathBuf>)
                     -> EngineResult<Vec<StratBlockDev>> {
    // Map blockdev UUIDs to pair of their position within
    // backstore_save.block_devs and their metadata.
    let recorded_data_map: HashMap<_, _> = backstore_save
        .block_devs
        .iter()
        .enumerate()
        .map(|(index, bds)| (bds.uuid, (index, bds)))
        .collect();

    // Sort the segments by the UUID to which they refer, i.e., group by
    // blockdev.
    let segments = &backstore_save.segments;
    let mut segment_table = HashMap::new();
    for seg in segments {
        segment_table
            .entry(seg.0)
            .or_insert_with(Vec::default)
            .push((seg.1, seg.2))
    }

    // Get sorted blockdevs
    let blockdevs = get_blockdevs_internal(pool_uuid, &recorded_data_map, &segment_table, devnodes)?;

    // Verify that blockdevs found match blockdevs recorded.
    let mut current_uuids = HashSet::new();
    for blockdev in &blockdevs {
        if !current_uuids.insert(blockdev.uuid()) {
            let err_msg = format!("Two devices w/ same UUID ({}) found in environment",
                                  blockdev.uuid());

            return Err(EngineError::Engine(ErrorEnum::Invalid, err_msg));
        }
    }

    let recorded_uuids: HashSet<_> = recorded_data_map.keys().cloned().collect();

    if current_uuids != recorded_uuids {
        assert!(current_uuids.len() < recorded_uuids.len(),
                "Returned with error if device UUID could not be found");
        let missing_uuids = recorded_uuids.sub(&current_uuids);
        let err_msg = format!("Blockdev UUIDs recorded in the metadata but not found: {}",
                              missing_uuids
                                  .iter()
                                  .map(|u| u.to_string())
                                  .collect::<Vec<_>>()
                                  .join(", "));
        return Err(EngineError::Engine(ErrorEnum::Invalid, err_msg));
    }

    Ok(blockdevs)
}

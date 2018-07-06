// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle initial setup steps for a pool.
// Initial setup steps are steps that do not alter the environment.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};

use libc::O_DIRECT;
use serde_json;

use devicemapper::{devnode_to_devno, Device};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::structures::Table;
use super::super::super::types::{Name, PoolUuid};

use super::super::pool::{check_metadata, StratPool};
use super::super::serde_structs::{BackstoreSave, PoolSave};

use super::blockdev::StratBlockDev;
use super::device::{blkdev_size, is_stratis_device};
use super::metadata::BDA;
use super::util::get_stratis_block_devices;

/// Setup a pool from constituent devices in the context of some already
/// setup pools. Return an error on anything that prevents the pool
/// being set up.
pub fn setup_pool(
    pool_uuid: PoolUuid,
    devices: &HashMap<Device, PathBuf>,
    pools: &Table<StratPool>,
) -> StratisResult<(Name, StratPool)> {
    // FIXME: In this method, various errors are assembled from various
    // sources and combined into strings, so that they
    // can be printed as log messages if necessary. Instead, some kind of
    // error-chaining should be used here and if it is necessary
    // to log the error, the log code should be able to reduce the error
    // chain to something that can be sensibly logged.
    let info_string = || {
        let dev_paths = devices
            .values()
            .map(|p| p.to_str().expect("Unix is utf-8"))
            .collect::<Vec<&str>>()
            .join(" ,");
        format!("(pool UUID: {}, devnodes: {})", pool_uuid, dev_paths)
    };

    let metadata = get_metadata(pool_uuid, devices)?.ok_or_else(|| {
        let err_msg = format!("no metadata found for {}", info_string());
        StratisError::Engine(ErrorEnum::NotFound, err_msg)
    })?;

    if pools.contains_name(&metadata.name) {
        let err_msg = format!(
            "pool with name \"{}\" set up; metadata specifies same name for {}",
            &metadata.name,
            info_string()
        );
        return Err(StratisError::Engine(ErrorEnum::AlreadyExists, err_msg));
    }

    check_metadata(&metadata)
        .or_else(|e| {
            let err_msg = format!(
                "inconsistent metadata for {}: reason: {:?}",
                info_string(),
                e
            );
            Err(StratisError::Engine(ErrorEnum::Error, err_msg))
        })
        .and_then(|_| {
            StratPool::setup(pool_uuid, devices, &metadata).or_else(|e| {
                let err_msg = format!(
                    "failed to set up pool for {}: reason: {:?}",
                    info_string(),
                    e
                );
                Err(StratisError::Engine(ErrorEnum::Error, err_msg))
            })
        })
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
        let bda = BDA::load(&mut OpenOptions::new()
            .read(true)
            .custom_flags(O_DIRECT)
            .open(devnode)?)?;
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
/// Returns an error if the blockdevs obtained do not match the metadata.
/// Returns a tuple, of which the first are the data devs, and the second
/// are the devs that support the cache tier.
#[allow(implicit_hasher)]
pub fn get_blockdevs(
    pool_uuid: PoolUuid,
    backstore_save: &BackstoreSave,
    devnodes: &HashMap<Device, PathBuf>,
) -> StratisResult<(Vec<StratBlockDev>, Vec<StratBlockDev>)> {
    let recorded_data_map: HashMap<_, _> = backstore_save
        .data_devs
        .iter()
        .map(|bds| (bds.uuid, bds))
        .collect();

    let recorded_cache_map: HashMap<_, _> = match backstore_save.cache_devs {
        Some(ref cache_devs) => cache_devs.iter().map(|bds| (bds.uuid, bds)).collect(),
        None => HashMap::new(),
    };

    let mut segment_table = HashMap::new();
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

    let (mut datadevs, mut cachedevs) = (vec![], vec![]);
    for (device, devnode) in devnodes {
        let bda = BDA::load(&mut OpenOptions::new()
            .read(true)
            .custom_flags(O_DIRECT)
            .open(devnode)?)?;
        if let Some(bda) = bda {
            if bda.pool_uuid() == pool_uuid {
                let actual_size =
                    blkdev_size(&OpenOptions::new().read(true).open(devnode)?)?.sectors();

                if actual_size < bda.dev_size() {
                    let err_msg = format!(
                        "actual blockdev size ({}) < recorded size ({})",
                        actual_size,
                        bda.dev_size()
                    );

                    return Err(StratisError::Engine(ErrorEnum::Error, err_msg));
                }

                let dev_uuid = bda.dev_uuid();

                let (dev_vec, bd_save) = match recorded_data_map.get(&dev_uuid) {
                    Some(bd_save) => (&mut datadevs, bd_save),
                    None => match recorded_cache_map.get(&dev_uuid) {
                        Some(bd_save) => (&mut cachedevs, bd_save),
                        None => {
                            let err_msg =
                                format!("Blockdev {} not found in metadata", bda.dev_uuid());
                            return Err(StratisError::Engine(ErrorEnum::NotFound, err_msg));
                        }
                    },
                };

                // This should always succeed since the actual size is at
                // least the recorded size, so all segments should be
                // available to be allocated. If this fails, the most likely
                // conclusion is metadata corruption.
                let segments = segment_table.get(&dev_uuid);
                dev_vec.push(StratBlockDev::new(
                    *device,
                    devnode.to_owned(),
                    bda,
                    segments.unwrap_or(&vec![]),
                    bd_save.user_info.clone(),
                    bd_save.hardware_info.clone(),
                )?);
            }
        }
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

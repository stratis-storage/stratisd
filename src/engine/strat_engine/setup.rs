// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle initial setup steps for a pool.
// Initial setup steps are steps that do not alter the environment.

use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::fs::{OpenOptions, read_dir};
use std::os::linux::fs::MetadataExt;
use std::path::PathBuf;
use std::str::FromStr;

use nix::Errno;
use nix::sys::stat::{S_IFBLK, S_IFMT};
use serde_json;

use devicemapper::Device;

use super::super::errors::{EngineResult, EngineError, ErrorEnum};
use super::super::types::PoolUuid;

use super::blockdev::BlockDev;
use super::device::blkdev_size;
use super::engine::DevOwnership;
use super::metadata::{BDA, StaticHeader};
use super::range_alloc::RangeAllocator;
use super::serde_structs::PoolSave;


/// Find all Stratis devices.
///
/// Returns a map of pool uuids to a vector of devices for each pool.
pub fn find_all() -> EngineResult<HashMap<PoolUuid, Vec<PathBuf>>> {

    let mut pool_map = HashMap::new();
    for dir_e in try!(read_dir("/dev")) {
        let dir_e = try!(dir_e);
        let mode = try!(dir_e.metadata()).st_mode();

        // Device node can't belong to Stratis if it is not a block device
        if mode & S_IFMT.bits() != S_IFBLK.bits() {
            continue;
        }

        let devnode = dir_e.path();

        let f = OpenOptions::new().read(true).open(&devnode);

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
        if f.is_err() {
            let err = f.unwrap_err();
            match err.kind() {
                ErrorKind::NotFound => {
                    continue;
                }
                _ => {
                    if let Some(errno) = err.raw_os_error() {
                        match Errno::from_i32(errno) {
                            Errno::ENXIO | Errno::ENOMEDIUM => continue,
                            _ => return Err(EngineError::Io(err)),
                        };
                    } else {
                        return Err(EngineError::Io(err));
                    }
                }
            }
        }

        let mut f = f.expect("unreachable if f is err");
        if let DevOwnership::Ours(uuid) = try!(StaticHeader::determine_ownership(&mut f)) {
            pool_map
                .entry(uuid)
                .or_insert_with(Vec::new)
                .push(devnode)
        };
    }

    Ok(pool_map)
}

/// Get the most recent metadata from a set of Devices for a given pool UUID.
/// Returns None if no metadata found for this pool.
pub fn get_metadata(pool_uuid: PoolUuid, devnodes: &[PathBuf]) -> EngineResult<Option<PoolSave>> {

    // Get pairs of device nodes and matching BDAs from readable and
    // valid devices. Skip if no BDA, or BDA UUID does not match pool
    // UUID.
    let bdas = devnodes
        .iter()
        .filter_map(|devnode| {
            OpenOptions::new()
                .read(true)
                .open(devnode)
                .ok()
                .and_then(|mut f| BDA::load(&mut f).ok())
                .and_then(|opt| opt)
                .and_then(|bda| if *bda.pool_uuid() == pool_uuid {
                              Some((devnode, bda))
                          } else {
                              None
                          })
        })
        .collect::<Vec<_>>();

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
    let err_str = "timestamp indicates data was written, but no data succesfully read";
    Err(EngineError::Engine(ErrorEnum::NotFound, err_str.into()))
}

/// Get the blockdevs corresponding to this pool.
pub fn get_blockdevs(pool_save: &PoolSave, devnodes: &[PathBuf]) -> EngineResult<Vec<BlockDev>> {
    let segments = pool_save
        .flex_devs
        .meta_dev
        .iter()
        .chain(pool_save.flex_devs.thin_meta_dev.iter())
        .chain(pool_save.flex_devs.thin_data_dev.iter());

    let mut segment_table = HashMap::new();
    for seg in segments {
        segment_table
            .entry(seg.0.clone())
            .or_insert(vec![])
            .push((seg.1, seg.2))
    }

    let mut blockdevs = vec![];
    let mut devices = HashSet::new();
    for dev in devnodes {
        let device = try!(Device::from_str(&dev.to_string_lossy()));

        // If we've seen this device already, skip it.
        if !devices.insert(device) {
            continue;
        }

        let bda = try!(BDA::load(&mut try!(OpenOptions::new().read(true).open(dev))));
        let bda = try!(bda.ok_or(EngineError::Engine(ErrorEnum::NotFound,
                                                     "no BDA found for Stratis device".into())));

        let actual_size = try!(blkdev_size(&try!(OpenOptions::new().read(true).open(dev))))
            .sectors();

        // If size of device has changed and is less, then it is possible
        // that the segments previously allocated for this blockdev no
        // longer exist. If that is the case, RangeAllocator::new() will
        // return an error.
        let allocator =
            try!(RangeAllocator::new(actual_size,
                                     segment_table.get(bda.dev_uuid()).unwrap_or(&vec![])));

        blockdevs.push(BlockDev::new(device, dev.clone(), bda, allocator));
    }

    // Verify that blockdevs found match blockdevs recorded.
    let current_uuids: HashSet<_> = blockdevs.iter().map(|b| *b.uuid()).collect();
    let recorded_uuids: HashSet<_> = pool_save.block_devs.keys().map(|u| *u).collect();

    if current_uuids != recorded_uuids {
        let err_msg = "Recorded block dev UUIDs != discovered blockdev UUIDs";
        return Err(EngineError::Engine(ErrorEnum::Invalid, err_msg.into()));
    }

    if blockdevs.len() != current_uuids.len() {
        let err_msg = "Duplicate block devices found in environment";
        return Err(EngineError::Engine(ErrorEnum::Invalid, err_msg.into()));
    }

    Ok(blockdevs)
}

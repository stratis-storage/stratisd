// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a single block device.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::fs::{OpenOptions, read_dir};
use std::os::linux::fs::MetadataExt;
use std::path::PathBuf;

use nix::Errno;
use nix::sys::stat::{S_IFBLK, S_IFMT};
use serde_json;

use engine::{EngineResult, EngineError, ErrorEnum, PoolUuid};
use super::metadata::{BDA, StaticHeader};
use super::engine::DevOwnership;
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
        // 1. The device does not exist anymore. This means that the device
        // was volatile for some reason; in that case it can not belong to
        // Stratis so it is safe to ignore it.
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
                        if Errno::from_i32(errno) == Errno::ENXIO {
                            continue;
                        } else {
                            return Err(EngineError::Io(err));
                        }
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
pub fn get_metadata(pool_uuid: &PoolUuid, devnodes: &[PathBuf]) -> EngineResult<Option<PoolSave>> {

    // No device nodes means no metadata
    if devnodes.is_empty() {
        return Ok(None);
    }

    // Get pairs of device nodes and matching BDAs
    // If no BDA, or BDA UUID does not match pool UUID, skip.
    // If there is an error reading the BDA, error. There could have been
    // vital information on that BDA, for example, it may have contained
    // the newest metadata.
    let mut bdas = Vec::new();
    for devnode in devnodes {
        let bda = try!(BDA::load(&mut try!(OpenOptions::new().read(true).open(devnode))));
        if bda.is_none() {
            continue;
        }
        let bda = bda.expect("unreachable if bda is None");

        if bda.pool_uuid() != pool_uuid {
            continue;
        }
        bdas.push((devnode, bda));
    }

    // We may have had no devices with BDAs for this pool, so return if no BDAs.
    if bdas.is_empty() {
        return Ok(None);
    }

    // Get a most recent BDA
    let &(_, ref most_recent_bda) = bdas.iter()
        .max_by_key(|p| p.1.last_update_time())
        .expect("bdas is not empty, must have a max");

    // Most recent time should never be None if this was a properly
    // created pool; this allows for the method to be called in other
    // circumstances.
    let most_recent_time = most_recent_bda.last_update_time();
    if most_recent_time.is_none() {
        return Ok(None);
    }

    // Try to read from all available devnodes that could contain most
    // recent metadata. In the event of errors, continue to try until all are
    // exhausted.
    for &(devnode, ref bda) in
        bdas.iter()
            .filter(|p| p.1.last_update_time() == most_recent_time) {

        let f = OpenOptions::new().read(true).open(devnode);
        if f.is_err() {
            continue;
        }
        let mut f = f.expect("f is not err");

        if let Ok(Some(data)) = bda.load_state(&mut f) {
            let json: serde_json::Result<PoolSave> = serde_json::from_slice(&data);
            if let Ok(pool) = json {
                return Ok(Some(pool));
            } else {
                continue;
            }
        } else {
            continue;
        }
    }

    // If no data has yet returned, we have an error. That is, we should have
    // some metadata, because we have a most recent time, but we failed to
    // get any.
    let err_str = "timestamp indicates data was written, but no data succesfully read";
    Err(EngineError::Engine(ErrorEnum::NotFound, err_str.into()))
}


/// Get the most recent metadata for each pool.
/// Since the metadata is written immediately after a pool is created, it
/// is considered an error for a pool to be w/out metadata.
pub fn get_pool_metadata(pool_table: &HashMap<PoolUuid, Vec<PathBuf>>)
                         -> EngineResult<HashMap<PoolUuid, PoolSave>> {
    let mut metadata = HashMap::new();
    for (pool_uuid, devices) in pool_table.iter() {
        if let Ok(Some(pool)) = get_metadata(pool_uuid, &devices) {
            metadata.insert(pool_uuid.clone(), pool);
        } else {
            return Err(EngineError::Engine(ErrorEnum::NotFound,
                                           format!("no metadata for pool {}", pool_uuid)));
        }
    }
    Ok(metadata)
}

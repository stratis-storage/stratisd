// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a single block device.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::fs::{OpenOptions, read_dir};
use std::os::linux::fs::MetadataExt;
use std::str::FromStr;

use devicemapper::Device;
use nix::Errno;
use nix::sys::stat::{S_IFBLK, S_IFMT};

use engine::{EngineResult, EngineError, ErrorEnum, PoolUuid};
use super::metadata::StaticHeader;
use super::engine::DevOwnership;


/// Find all Stratis devices.
///
/// Returns a map of pool uuids to a vector of devices for each pool.
pub fn find_all() -> EngineResult<HashMap<PoolUuid, Vec<Device>>> {

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
        // 1. The device does not exist anymore.
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

        match try!(StaticHeader::determine_ownership(&mut f)) {
            DevOwnership::Ours(uuid) => {
                let dev = try!(Device::from_str(&devnode.to_string_lossy()));
                pool_map.entry(uuid).or_insert_with(Vec::new).push(dev)
            }
            _ => continue,
        };
    }

    Ok(pool_map)
}

/// Get the most recent metadata from a set of Devices.
/// Precondition: All devices belong to the same pool.
pub fn get_metadata(pool_uuid: &PoolUuid, devices: &[Device]) -> Option<Vec<u8>> {
    unimplemented!()
}


/// Get the most recent metadata for each pool.
/// Since the metadata is written immediately after a pool is created, it
/// is considered an error for a pool to be w/out metadata.
pub fn get_pool_metadata(pool_table: &HashMap<PoolUuid, Vec<Device>>)
                         -> EngineResult<HashMap<PoolUuid, Vec<u8>>> {
    let mut metadata = HashMap::new();
    for (pool_uuid, devices) in pool_table.iter() {
        if let Some(bytes) = get_metadata(pool_uuid, devices.as_slice()) {
            metadata.insert(pool_uuid.clone(), bytes);
        } else {
            return Err(EngineError::Engine(ErrorEnum::NotFound,
                                           format!("no metadata for pool {}", pool_uuid)));
        }
    }
    Ok(metadata)
}

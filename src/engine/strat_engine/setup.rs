// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;
use std::fs::{OpenOptions, read_dir};
use std::io::ErrorKind;
use std::os::linux::fs::MetadataExt;
use std::path::Path;
use std::str::FromStr;

use nix::sys::stat::{S_IFBLK, S_IFMT};
use nix::Errno;

use devicemapper::{Device, Sectors};

use engine::{DevUuid, EngineError, EngineResult, PoolUuid};

use super::blockdev::BlockDev;
use super::metadata::BDA;
use super::range_alloc::RangeAllocator;


/// Find all Stratis Blockdevs.
///
/// Returns a map of pool uuids to maps of blockdev uuids to blockdevs.
pub fn find_all() -> EngineResult<HashMap<PoolUuid, HashMap<DevUuid, BlockDev>>> {

    /// If a Path refers to a valid Stratis blockdev, return a BlockDev
    /// struct. Otherwise, return None. Return an error if there was
    /// a problem inspecting the device.
    fn setup(devnode: &Path) -> EngineResult<Option<BlockDev>> {
        let f = OpenOptions::new().read(true).open(devnode);

        // There are some reasons for OpenOptions::open() to return an error
        // which are not reasons for this method to return an error.
        // Try to distinguish between these in case there is an error.
        // Non-error conditions are:
        // 1. The device is not found.
        if f.is_err() {
            let err = f.unwrap_err();
            return match err.kind() {
                ErrorKind::NotFound => Ok(None),
                _ => {
                    if let Some(errno) = err.raw_os_error() {
                        if Errno::from_i32(errno) == Errno::ENXIO {
                            Ok(None)
                        } else {
                            Err(EngineError::Io(err))
                        }
                    } else {
                        Err(EngineError::Io(err))
                    }
                }
            };
        }

        let mut f = f.expect("f must be ok, since method returns if f is err");

        if let Some(bda) = BDA::load(&mut f).ok() {
            let dev = try!(Device::from_str(&devnode.to_string_lossy()));

            // TODO: Parse MDA and also initialize RangeAllocator with
            // in-use regions
            let allocator = RangeAllocator::new_with_used(bda.dev_size(),
                                                          &[(Sectors(0), bda.size())]);
            Ok(Some(BlockDev::new(dev, devnode, bda, allocator)))
        } else {
            Ok(None)
        }
    }

    let mut pool_map = HashMap::new();
    for dir_e in try!(read_dir("/dev")) {
        let dir_e = try!(dir_e);
        let mode = try!(dir_e.metadata()).st_mode();

        // Device node can't belong to Stratis if it is not a block device
        if mode & S_IFMT.bits() != S_IFBLK.bits() {
            continue;
        }

        if let Some(blockdev) = try!(setup(&dir_e.path())) {
            pool_map.entry(blockdev.pool_uuid().clone())
                .or_insert_with(HashMap::new)
                .insert(blockdev.uuid().clone(), blockdev);
        }
    }

    Ok(pool_map)
}

/// Return the metadata from the first blockdev with up-to-date, readable
/// metadata.
/// Precondition: All BlockDevs in blockdevs must belong to the same pool.
pub fn load_state(blockdevs: &[&BlockDev]) -> Option<Vec<u8>> {
    if blockdevs.is_empty() {
        return None;
    }

    let most_recent_blockdev = blockdevs.iter()
        .max_by_key(|bd| bd.last_update_time())
        .expect("must be a maximum since bds is non-empty");

    let most_recent_time = most_recent_blockdev.last_update_time();

    if most_recent_time.is_none() {
        return None;
    }

    for bd in blockdevs.iter()
        .filter(|b| b.last_update_time() == most_recent_time) {
        match bd.load_state() {
            Ok(Some(data)) => return Some(data),
            _ => continue,
        }
    }

    None
}

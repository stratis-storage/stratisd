// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;
use std::fs::{OpenOptions, read_dir};
use std::path::Path;
use std::str::FromStr;

use devicemapper::{Device, Sectors};

use engine::{DevUuid, EngineResult, PoolUuid};

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
        let mut f = try!(OpenOptions::new()
            .read(true)
            .open(devnode));

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
        let devnode = try!(dir_e.map(|d| d.path()));

        if let Some(blockdev) = try!(setup(&devnode)) {
            pool_map.entry(blockdev.pool_uuid().clone())
                .or_insert_with(HashMap::new)
                .insert(blockdev.uuid().clone(), blockdev);
        }
    }

    Ok(pool_map)
}

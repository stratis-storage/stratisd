// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;
use std::fs::{OpenOptions, read_dir};
use std::path::Path;
use std::str::FromStr;

use devicemapper::Device;

use engine::{DevUuid, EngineResult, PoolUuid};

use super::blockdev::BlockDev;
use super::metadata::BDA;


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
            Ok(Some(BlockDev::new(dev, devnode, bda)))
        } else {
            Ok(None)
        }
    }

    let mut pool_map = HashMap::new();
    for dir_e in try!(read_dir("/dev")) {
        let devnode = match dir_e {
            Ok(d) => d.path(),
            Err(_) => continue,
        };

        match setup(&devnode) {
            Ok(Some(blockdev)) => {
                pool_map.entry(blockdev.pool_uuid().clone())
                    .or_insert_with(HashMap::new)
                    .insert(blockdev.uuid().clone(), blockdev);
            }
            _ => continue,
        };
    }

    Ok(pool_map)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Just run find all and make sure that it returns with an OK result.
    /// It is not running w/ root permission, so it should be relatively
    /// harmless.
    #[test]
    fn just_run() {
        assert!(find_all().is_ok());
    }
}

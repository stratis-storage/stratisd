// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::BTreeSet;
use std::path::Path;
use std::str::FromStr;

use uuid::Uuid;

use common::*;

use devicemapper::Device;

use stratisd::engine::strat_engine::blockdev::BlockDev;
use stratisd::engine::strat_engine::consts::MIN_MDA_SIZE;

pub fn test_blockdev_force_flag(blockdev_paths: &Vec<&Path>) -> TestResult<()> {

    let mut devices = BTreeSet::new();
    let mut devices_copy = BTreeSet::new();
    for path in blockdev_paths {
        let dev = try!(Device::from_str(&path.to_string_lossy()));
        devices.insert(dev);
        devices_copy.insert(dev.clone());
    }

    // Initialzie devices with force = ture
    let fake_pool_uuid = Uuid::new_v4();
    try!(BlockDev::initialize(&fake_pool_uuid, devices, MIN_MDA_SIZE, true));

    // Try to initialzie again with different uuid, force = false - this should fail
    let new_pool_uuid = Uuid::new_v4();
    match BlockDev::initialize(&new_pool_uuid, devices_copy, MIN_MDA_SIZE, false) {
        Ok(_) => {
            error!("initialize of already initialized blockdevs succeeded without force flag");
            return Err(TestError::Framework(TestErrorEnum::Error("initialize of already \
                                                                  initialized blockdevs \
                                                                  succeeded without force flag"
                .into())));
        }
        Err(_) => {
            info!("PASS: initialize of already initialzied blockdevs failed (intended)");
        }
    }

    Ok(())
}

pub fn test_blockdevs(blockdev_paths: &Vec<&Path>) -> TestResult<()> {
    info!("Starting BlockDev Tests...");
    let mut devices = BTreeSet::new();
    for path in blockdev_paths {
        let dev = try!(Device::from_str(&path.to_string_lossy()));
        devices.insert(dev);
    }

    try!(test_blockdev_force_flag(blockdev_paths));

    info!("BlockDev Tests Complete.");
    Ok(())
}

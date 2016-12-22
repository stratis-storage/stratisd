// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;
use std::path::PathBuf;

use stratisd::engine::strat_engine::StratEngine;
use stratisd::engine::Engine;

use common::*;

pub fn validate_disks_released(_blockdev_paths: &Vec<PathBuf>) -> TestResult<()> {

    // TODO loop through devs and validate headers have been erased
    Ok(())
}

pub fn validate_disks_init(_blockdev_paths: &Vec<PathBuf>) -> TestResult<()> {

    // TODO loop through devs and validate headers
    Ok(())
}

pub fn test_create_delete(blockdev_paths: &Vec<&Path>) -> TestResult<()> {

    let pool_name = "test_pool";

    let mut engine = StratEngine::new();

    let blockdevs = try!(engine.create_pool(pool_name, blockdev_paths, 1, true));

    try!(validate_disks_init(&blockdevs));

    try!(engine.destroy_pool(pool_name));

    try!(validate_disks_released(&blockdevs));

    Ok(())
}

pub fn test_pools(blockdev_paths: &Vec<&Path>) -> TestResult<()> {

    info!("Starting Pool Tests...");
    try!(test_create_delete(blockdev_paths));
    info!("Pool Tests Complete.");
    Ok(())
}

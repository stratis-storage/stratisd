// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
#[macro_use]
extern crate log;
extern crate uuid;
extern crate devicemapper;
extern crate stratis;
#[macro_use]
mod util;

use stratis::engine::strat_engine::blockdev;
use stratis::engine::strat_engine::metadata::MIN_MDA_SECTORS;

use std::path::Path;

use util::blockdev_utils::clean_blockdev_headers;
use util::test_config::TestConfig;
use util::test_consts::DEFAULT_CONFIG_FILE;
use util::test_result::TestError::Framework;
use util::test_result::TestErrorEnum::Error;
use util::test_result::TestResult;

use uuid::Uuid;

// Test to make sure an initialized blockdev can't be re-initialized without
// the force flag.
pub fn test_blockdev_force_flag(blockdev_paths: &Vec<&Path>) -> TestResult<()> {

    let unique_devices = match blockdev::resolve_devices(blockdev_paths) {
        Ok(devs) => devs,
        Err(e) => {
            let message = format!("Failed to resolve blockdevs: {:?}", e);
            return Err(Framework(Error(message)));
        }
    };

    let devices_copy = unique_devices.clone();

    // Initialzie devices with force = ture
    match blockdev::initialize(&Uuid::new_v4(), unique_devices, MIN_MDA_SECTORS, true) {
        Ok(_) => {
            debug!("Initialzied starting set of devices");
        }
        Err(e) => {
            let message = format!("Failed to initialize starting set of devices: {:?}", e);
            return Err(Framework(Error(message)));
        }
    }

    // Try to initialzie again with different uuid, force = false - this should fail
    match blockdev::initialize(&Uuid::new_v4(), devices_copy, MIN_MDA_SECTORS, false) {
        Ok(_) => {
            let message = format!("initialize of already initialized blockdevs succeeded \
                                        without force flag");
            return Err(Framework(Error(message)));
        }
        Err(_) => {
            info!("PASS: initialize of already initialzied blockdevs failed (intended)");
        }
    }

    Ok(())
}

#[test]
pub fn test_blockdev_setup() {
    let mut test_config = TestConfig::new(DEFAULT_CONFIG_FILE);

    let _ = test_config.init();

    let safe_to_destroy_devs = match test_config.get_safe_to_destroy_devs() {
        Ok(devs) => {
            if devs.len() == 0 {
                warn!("No devs availabe for testing.  Test not run");
                return;
            }
            devs
        }
        Err(e) => {
            error!("Failed : get_safe_to_destroy_devs : {:?}", e);
            return;
        }
    };

    info!("safe_to_destroy_devs = {:?}", safe_to_destroy_devs);

    let device_paths = safe_to_destroy_devs.iter().map(|x| Path::new(x)).collect::<Vec<&Path>>();

    clean_blockdev_headers(&device_paths);

    assert!(match test_blockdev_force_flag(&device_paths) {
        Ok(_) => true,
        Err(e) => {
            error!("Failed : test_blockdev_force_flag : {:?}", e);
            false
        }
    });

}

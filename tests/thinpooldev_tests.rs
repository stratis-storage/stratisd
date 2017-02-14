// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
#[macro_use]
extern crate log;
extern crate uuid;
extern crate devicemapper;
extern crate libstratis;
#[macro_use]
mod util;

use devicemapper::DM;
use libstratis::engine::strat_engine::blockdev;
use libstratis::engine::strat_engine::metadata::MIN_MDA_SECTORS;
use libstratis::engine::strat_engine::thinpooldev::ThinPoolDev;
use libstratis::types::DataBlocks;
use libstratis::types::Sectors;

use std::path::Path;

use util::blockdev_utils::clean_blockdev_headers;
use util::blockdev_utils::get_size;
use util::test_config::TestConfig;
use util::test_consts::DEFAULT_CONFIG_FILE;
use util::test_result::TestError::Framework;
use util::test_result::TestErrorEnum::Error;
use util::test_result::TestResult;

use uuid::Uuid;

/// Validate the blockdev_paths are unique
/// Initialize the list for use with Stratis
/// Create a thin-pool via ThinPoolDev
/// Validate the resulting thin-pool dev and meta dev
fn test_thinpool_setup(dm: &DM,
                       thinpool_dev: &mut ThinPoolDev,
                       blockdev_paths: &Vec<&Path>)
                       -> TestResult<()> {

    let uuid = Uuid::new_v4();

    let unique_blockdevs = match blockdev::resolve_devices(blockdev_paths) {
        Ok(devs) => devs,
        Err(e) => {
            let message = format!("Failed to resolve starting set of blockdevs:{:?}", e);
            return Err(Framework(Error(message)));
        }
    };

    let blockdev_map = match blockdev::initialize(&uuid, unique_blockdevs, MIN_MDA_SECTORS, true) {
        Ok(blockdev_map) => blockdev_map,
        Err(e) => {
            let message = format!("Failed to initialize starting set of blockdevs {:?}", e);
            return Err(Framework(Error(message)));
        }
    };

    let (_first_key, metadata_blockdev) = blockdev_map.iter().next().unwrap();
    let (_last_key, data_blockdev) = blockdev_map.iter().next_back().unwrap();
    let (_start_sector, length) = data_blockdev.avail_range();

    match thinpool_dev.setup(dm,
                             &length,
                             &Sectors(1024),
                             &DataBlocks(256000),
                             metadata_blockdev,
                             data_blockdev) {
        Ok(_) => info!("completed test on {:?}", thinpool_dev.name),
        Err(e) => {
            let message = format!("Failed to initialize starting set of blockdevs {:?}", e);
            return Err(Framework(Error(message)));
        }
    }

    Ok(())
}

#[test]
/// Get list of safe to destroy devices.
/// Clean any headers from the devices.
/// Test creating a thin-pool device
/// Teardown the DM device
pub fn test_thinpoolsetup_setup() {

    let dm = DM::new().unwrap();

    let mut test_config = TestConfig::new(DEFAULT_CONFIG_FILE);

    let _ = test_config.init();

    let safe_to_destroy_devs = match test_config.get_safe_to_destroy_devs() {
        Ok(devs) => {
            if devs.len() < 2 {
                warn!("test_thinpoolsetup_setup requires at least 2 devices to run.  Test not \
                       run.");
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
    info!("devices cleaned for test");

    let name = "stratis_testing_thinpool";

    let mut thinpool_dev = ThinPoolDev::new(name);

    match test_thinpool_setup(&dm, &mut thinpool_dev, &device_paths) {
        Ok(_) => {
            info!("completed test on {}", name);
            true
        }
        Err(e) => {
            error!("Failed : test_thinpoolsetup_setup : {:?}", e);
            false
        }
    };

    match thinpool_dev.teardown(&dm) {
        Ok(_) => info!("completed teardown of {}", name),
        Err(e) => panic!("Failed to teardown {} : {:?}", name, e),
    }

}

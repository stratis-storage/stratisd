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
use libstratis::engine::strat_engine::blockdev::BlockDev;
use libstratis::engine::strat_engine::device::resolve_devices;
use libstratis::engine::strat_engine::lineardev::LinearDev;
use libstratis::engine::strat_engine::metadata::MIN_MDA_SECTORS;
use libstratis::types::Sectors;

use std::iter::FromIterator;
use std::path::Path;

use util::blockdev_utils::clean_blockdev_headers;
use util::blockdev_utils::get_size;
use util::test_config::TestConfig;
use util::test_consts::DEFAULT_CONFIG_FILE;
use util::test_result::TestError::Framework;
use util::test_result::TestErrorEnum::Error;
use util::test_result::TestResult;

use uuid::Uuid;

/// Return a LinearDev with concatenated BlockDevs
fn concat_blockdevs(dm: &DM, name: &str, block_devs: &[&BlockDev]) -> TestResult<LinearDev> {

    match LinearDev::new(name, dm, block_devs) {
        Ok(ld) => return Ok(ld),
        Err(e) => {
            let message = format!("linear_dev.concat failed : {:?}", e);
            return Err(Framework(Error(message)));
        }
    }
}

/// Get the usable sector lengths for each dev
/// Wait for the DM device to be created
/// Validate the size of the DM device with the sum of the sector lengths
fn validate_sizes(name: &str, block_devs: &[&BlockDev]) -> TestResult<()> {

    let mut linear_sectors = Sectors(0);

    for blockdev in block_devs {
        let (_start_sector, length) = blockdev.avail_range();
        linear_sectors = linear_sectors + length;
    }

    debug!("available linear space = {} sectors", linear_sectors);

    let path_name = format!("/dev/mapper/{}", name);
    let path = Path::new(&path_name);

    let dm_dev_size = match get_size(path) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to get size for {} ", name);
            return Err(e);
        }
    };
    debug!("size of dm device {:?} = {}", path, dm_dev_size);

    assert_eq!(linear_sectors, dm_dev_size);

    Ok(())
}

/// Validate the blockdev_paths are unique
/// Initialize the list for use with Stratis
/// Concatenate the list via LinearDev
/// Validate the size of the resulting DM device
fn test_lineardev_concat(dm: &DM, blockdev_paths: &[&Path]) -> TestResult<(LinearDev)> {

    let uuid = Uuid::new_v4();

    let unique_blockdevs = match resolve_devices(blockdev_paths) {
        Ok(devs) => devs,
        Err(e) => {
            let message = format!("Failed to resolve starting set of blockdevs:{:?}", e);
            return Err(Framework(Error(message)));
        }
    };

    let blockdevs = match blockdev::initialize(&uuid, unique_blockdevs, MIN_MDA_SECTORS, true) {
        Ok(blockdevs) => blockdevs,
        Err(e) => {
            let message = format!("Failed to initialize starting set of blockdevs {:?}", e);
            return Err(Framework(Error(message)));
        }
    };

    let blockdev_refs = Vec::from_iter(blockdevs.iter());
    let name = "stratis_testing_linear";
    let linear_dev = match concat_blockdevs(dm, &name, &blockdev_refs) {
        Ok(dev) => dev,
        Err(e) => {
            let message = format!("Failed to concat_blockdevs {:?}", e);
            return Err(Framework(Error(message)));
        }
    };

    match validate_sizes(&name, &blockdev_refs) {
        Ok(_) => info!("validate_sizes Ok"),
        Err(e) => {
            error!("Failed : validate_sizes() : {:?}", e);
            return Err(e);
        }
    }

    Ok(linear_dev)
}

#[test]
/// Get list of safe to destroy devices.
/// Clean any headers from the devices.
/// Test concatenating the list of devices into linear sectors
/// Teardown the DM device
pub fn test_lineardev_setup() {

    let dm = DM::new().unwrap();

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

    assert_ok!(clean_blockdev_headers(&device_paths));

    info!("devices cleaned for test");

    assert!(match test_lineardev_concat(&dm, &device_paths) {
        Ok(linear_dev) => {
            info!("completed test on {}", linear_dev.name());

            match linear_dev.teardown(&dm) {
                Ok(_) => info!("completed teardown of {}", linear_dev.name()),
                Err(e) => error!("Failed to teardown {} : {:?}", linear_dev.name(), e),
            }
            true
        }
        Err(e) => {
            error!("Failed : test_lineardev_concat : {:?}", e);
            false
        }
    });


}

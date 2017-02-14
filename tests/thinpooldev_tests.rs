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
use libstratis::engine::strat_engine::lineardev::LinearDev;
use libstratis::engine::strat_engine::metadata::MIN_MDA_SECTORS;
use libstratis::engine::strat_engine::thinpooldev::ThinPoolDev;
use libstratis::types::DataBlocks;
use libstratis::types::Sectors;

use std::path::{Path, PathBuf};

use util::blockdev_utils::clean_blockdev_headers;
use util::blockdev_utils::wait_for_dm;
use util::test_config::TestConfig;
use util::test_consts::DEFAULT_CONFIG_FILE;
use util::test_result::TestError::Framework;
use util::test_result::TestErrorEnum::Error;
use util::test_result::TestResult;

use uuid::Uuid;

fn setup_supporting_devs(dm: &DM,
                         metadata_dev: &mut LinearDev,
                         data_dev: &mut LinearDev,
                         metadata_blockdev: &BlockDev,
                         data_blockdev: &BlockDev)
                         -> TestResult<(PathBuf, PathBuf)> {

    let mut meta_blockdevs = Vec::new();
    meta_blockdevs.push(metadata_blockdev);

    match metadata_dev.concat(dm, &meta_blockdevs) {
        Ok(_) => {}
        Err(e) => {
            let message = format!("metadata.concat failed : {:?}", e);
            return Err(Framework(Error(message)));
        }

    }

    let mut data_blockdevs = Vec::new();
    data_blockdevs.push(data_blockdev);


    match data_dev.concat(dm, &data_blockdevs) {
        Ok(_) => {}
        Err(e) => {
            let message = format!("datadev.concat failed : {:?}", e);
            return Err(Framework(Error(message)));
        }

    }

    let metadata_path = match metadata_dev.path() {
        Ok(path) => path.unwrap().clone(),
        Err(e) => {
            let message = format!("Failed to get data path : {:?}", e);
            return Err(Framework(Error(message)));
        }
    };

    let data_path = match data_dev.path() {
        Ok(path) => path.unwrap().clone(),
        Err(e) => {
            let message = format!("Failed to get data path : {:?}", e);
            return Err(Framework(Error(message)));
        }
    };

    wait_for_dm();

    Ok((metadata_path, data_path))
}
/// Validate the blockdev_paths are unique
/// Initialize the list for use with Stratis
/// Create a thin-pool via ThinPoolDev
/// Validate the resulting thin-pool dev and meta dev
fn test_thinpool_setup(dm: &DM,
                       thinpool_dev: &mut ThinPoolDev,
                       metadata_dev: &mut LinearDev,
                       data_dev: &mut LinearDev,
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

    let (metadata_path, data_path) =
        match setup_supporting_devs(dm, metadata_dev, data_dev, metadata_blockdev, data_blockdev) {
            Ok(tuple) => tuple,
            Err(e) => {
                let message = format!("Failed to setup_supporting_devs : {:?}", e);
                return Err(Framework(Error(message)));
            }
        };

    match thinpool_dev.setup(dm,
                             &length,
                             &Sectors(1024),
                             &DataBlocks(256000),
                             &metadata_path,
                             &data_path) {
        Ok(_) => info!("completed test on {:?}", thinpool_dev.name),
        Err(e) => {
            let message = format!("thinpool_dev.setup {:?}", e);
            return Err(Framework(Error(message)));
        }
    }

    wait_for_dm();

    Ok(())
}

#[test]
/// Get list of safe to destroy devices.
/// Clean any headers from the devices.
/// Construct meta and data devices for use in the thin-pool
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

    let meta_name = "stratis_testing_thinpool_metadata";
    let mut metadata_dev = LinearDev::new(meta_name);

    let name = "stratis_testing_thinpool";
    let mut thinpool_dev = ThinPoolDev::new(name);

    let data_name = "stratis_testing_thinpool_datadev";
    let mut data_dev = LinearDev::new(data_name);

    match test_thinpool_setup(&dm,
                              &mut thinpool_dev,
                              &mut metadata_dev,
                              &mut data_dev,
                              &device_paths) {
        Ok(_) => {
            info!("completed test on {}", name);
        }
        Err(e) => {
            error!("Failed : test_thinpoolsetup_setup : {:?}", e);
        }
    };

    match thinpool_dev.teardown(&dm) {
        Ok(_) => info!("completed teardown of {}", name),
        Err(e) => error!("Failed to teardown {} : {:?}", name, e),
    }

    match data_dev.teardown(&dm) {
        Ok(_) => info!("completed teardown of {}", data_name),
        Err(_) => error!("failed teardown of {}", data_name),
    }

    match metadata_dev.teardown(&dm) {
        Ok(_) => info!("completed teardown of {}", meta_name),
        Err(_) => error!("failed teardown of {}", meta_name),
    }
}

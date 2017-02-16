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

use uuid::Uuid;

fn setup_supporting_devs(dm: &DM,
                         metadata_dev: &mut LinearDev,
                         data_dev: &mut LinearDev,
                         metadata_blockdev: &BlockDev,
                         data_blockdev: &BlockDev)
                         -> (PathBuf, PathBuf) {

    let meta_blockdevs = vec![metadata_blockdev];
    metadata_dev.concat(dm, &meta_blockdevs).unwrap();

    let data_blockdevs = vec![data_blockdev];
    data_dev.concat(dm, &data_blockdevs).unwrap();

    let metadata_path = metadata_dev.path().unwrap().unwrap();

    let data_path = data_dev.path().unwrap().unwrap();

    wait_for_dm();

    (metadata_path, data_path)
}
/// Validate the blockdev_paths are unique
/// Initialize the list for use with Stratis
/// Create a thin-pool via ThinPoolDev
/// Validate the resulting thin-pool dev and meta dev
fn test_thinpool_setup(dm: &DM,
                       thinpool_dev: &mut ThinPoolDev,
                       metadata_dev: &mut LinearDev,
                       data_dev: &mut LinearDev,
                       blockdev_paths: &Vec<&Path>) {

    let uuid = Uuid::new_v4();

    let unique_blockdevs = blockdev::resolve_devices(blockdev_paths).unwrap();

    let blockdev_map = blockdev::initialize(&uuid, unique_blockdevs, MIN_MDA_SECTORS, true)
        .unwrap();

    let (_first_key, metadata_blockdev) = blockdev_map.iter().next().unwrap();
    let (_last_key, data_blockdev) = blockdev_map.iter().next_back().unwrap();
    let (_start_sector, length) = data_blockdev.avail_range();

    let (metadata_path, data_path) =
        setup_supporting_devs(dm, metadata_dev, data_dev, metadata_blockdev, data_blockdev);

    thinpool_dev.setup(dm,
               length,
               Sectors(1024),
               DataBlocks(256000),
               &metadata_path,
               &data_path)
        .unwrap();

    wait_for_dm();
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

    test_thinpool_setup(&dm,
                        &mut thinpool_dev,
                        &mut metadata_dev,
                        &mut data_dev,
                        &device_paths);

    thinpool_dev.teardown(&dm).unwrap();

    data_dev.teardown(&dm).unwrap();

    metadata_dev.teardown(&dm).unwrap();
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
#[macro_use]
extern crate log;
extern crate uuid;
extern crate devicemapper;
extern crate libstratis;
extern crate rand;
extern crate tempdir;
#[macro_use]
mod util;

use std::path::Path;

use devicemapper::DM;
use devicemapper::types::DataBlocks;
use devicemapper::types::Sectors;

use libstratis::engine::strat_engine::blockdev;
use libstratis::engine::strat_engine::blockdev::BlockDev;
use libstratis::engine::strat_engine::blockdev::wipe_sectors;
use libstratis::engine::strat_engine::lineardev::LinearDev;
use libstratis::engine::strat_engine::metadata::MIN_MDA_SECTORS;
use libstratis::engine::strat_engine::thindev::ThinDev;
use libstratis::engine::strat_engine::thinpooldev::ThinPoolDev;

use tempdir::TempDir;

use util::blockdev_utils::clean_blockdev_headers;
use util::blockdev_utils::write_files_to_directory;
use util::test_config::TestConfig;
use util::test_consts::DEFAULT_CONFIG_FILE;
use util::test_result::TestResult;

use uuid::Uuid;

fn setup_supporting_devs(dm: &DM,
                         metadata_blockdev: &BlockDev,
                         data_blockdev: &BlockDev)
                         -> TestResult<(LinearDev, LinearDev)> {

    let meta_blockdevs = vec![metadata_blockdev];
    let metadata_dev =
        try!(LinearDev::new("stratis_testing_thinpool_metadata", dm, &meta_blockdevs));

    let data_blockdevs = vec![data_blockdev];
    let data_dev = try!(LinearDev::new("stratis_testing_thinpool_datadev", dm, &data_blockdevs));

    let metadata_path = try!(metadata_dev.path());
    let data_path = try!(data_dev.path());
    try!(wipe_sectors(Path::new(&metadata_path), Sectors(0), Sectors(16)));
    try!(wipe_sectors(Path::new(&data_path), Sectors(0), Sectors(16)));

    Ok((metadata_dev, data_dev))
}

/// Validate the blockdev_paths are unique
/// Initialize the list for use with Stratis
/// Create a thin-pool via ThinPoolDev
/// Validate the resulting thin-pool dev and meta dev
fn test_thinpool_setup(dm: &DM, blockdev_paths: &[&Path]) -> TestResult<ThinPoolDev> {

    let uuid = Uuid::new_v4();

    let unique_blockdevs = blockdev::resolve_devices(blockdev_paths).unwrap();

    let blockdevs = blockdev::initialize(&uuid, unique_blockdevs, MIN_MDA_SECTORS, true).unwrap();
    let (metadata_blockdev, data_blockdev) = (blockdevs.first().unwrap(),
                                              blockdevs.last().unwrap());

    let (metadata_dev, data_dev) =
        try!(setup_supporting_devs(dm, metadata_blockdev, data_blockdev));

    let mut thinpool_dev = try!(ThinPoolDev::new("stratis_testing_thinpool",
                                                 dm,
                                                 data_blockdev.avail_range().1,
                                                 Sectors(1024),
                                                 DataBlocks(256000),
                                                 metadata_dev,
                                                 data_dev));


    try!(test_thindev_setup(&dm, &mut thinpool_dev));

    Ok(thinpool_dev)
}

fn test_thindev_setup(dm: &DM, thinpool_dev: &mut ThinPoolDev) -> TestResult<()> {
    let thin_id = rand::random::<u16>();
    let mut thin_dev = try!(ThinDev::new("stratis_testing_thindev",
                                         &dm,
                                         thinpool_dev,
                                         thin_id as u32,
                                         Sectors(300000)));


    let tmp_dir = try!(TempDir::new("stratis_testing"));

    try!(thin_dev.create_fs());
    try!(thin_dev.mount_fs(tmp_dir.path()));
    try!(write_files_to_directory(&tmp_dir, 100));

    try!(thin_dev.unmount_fs(tmp_dir.path()));
    try!(thin_dev.teardown(dm));
    Ok(())
}

#[test]
/// Get list of safe to destroy devices.
/// Clean any headers from the devices.
/// Construct meta and data devices for use in the thin-pool
/// Test creating a thin-pool device
/// Test create a thin device provisioned from the pool
/// Teardown the DM devices in the proper order
pub fn test_thinpool() {

    let dm = DM::new().unwrap();

    let mut test_config = TestConfig::new(DEFAULT_CONFIG_FILE);
    let _ = test_config.init();

    let safe_to_destroy_devs = match test_config.get_safe_to_destroy_devs() {
        Ok(devs) => {
            if devs.len() < 2 {
                warn!("test_thinpool requires at least 2 devices to run.  Test not \
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

    assert_ok!(clean_blockdev_headers(&device_paths));
    info!("devices cleaned for test");

    let thinpool_dev = assert_ok!(test_thinpool_setup(&dm, &device_paths));

    thinpool_dev.teardown(&dm).unwrap();
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Test the functionality of device mapper devices.


extern crate devicemapper;
extern crate libstratis;
extern crate tempdir;
extern crate uuid;

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use self::tempdir::TempDir;
use self::uuid::Uuid;

use self::devicemapper::DM;
use self::devicemapper::LinearDev;
use self::devicemapper::Segment;
use self::devicemapper::{DataBlocks, Sectors};
use self::devicemapper::{ThinDev, ThinDevId, ThinPoolDev};

use libstratis::engine::strat_engine::blockdevmgr::{initialize, resolve_devices};
use libstratis::engine::strat_engine::device::{blkdev_size, wipe_sectors};
use libstratis::engine::strat_engine::filesystem::{create_fs, mount_fs, unmount_fs};
use libstratis::engine::strat_engine::metadata::MIN_MDA_SECTORS;


/// Verify that the sum of the lengths of the available range of the
/// blockdevs in the linear device equals the size of the linear device.
pub fn test_linear_device(paths: &[&Path]) -> () {
    let unique_devices = resolve_devices(&paths).unwrap();
    let initialized = initialize(&Uuid::new_v4(),
                                 unique_devices.clone(),
                                 MIN_MDA_SECTORS,
                                 false)
            .unwrap();
    let total_blockdev_size = initialized
        .iter()
        .fold(Sectors(0), |s, i| s + i.avail_range().length);

    let segments = initialized
        .iter()
        .map(|block_dev| block_dev.avail_range())
        .collect::<Vec<Segment>>();

    let device_name = "stratis_testing_linear";
    let dm = DM::new().unwrap();
    let lineardev = LinearDev::new(&device_name, &dm, segments).unwrap();

    let mut linear_dev_path = PathBuf::from("/dev/mapper");
    linear_dev_path.push(device_name);

    let lineardev_size = blkdev_size(&OpenOptions::new()
                                          .read(true)
                                          .open(linear_dev_path)
                                          .unwrap())
            .unwrap();
    assert!(total_blockdev_size.bytes() == lineardev_size);
    lineardev.teardown(&dm).unwrap();
}


/// Verify no errors when writing files to a mounted filesystem on a thin
/// device.
pub fn test_thinpool_device(paths: &[&Path]) -> () {
    let initialized = initialize(&Uuid::new_v4(),
                                 resolve_devices(&paths).unwrap(),
                                 MIN_MDA_SECTORS,
                                 false)
            .unwrap();

    let (metadata_blockdev, data_blockdev) = (initialized.first().unwrap(),
                                              initialized.last().unwrap());

    let dm = DM::new().unwrap();
    let metadata_dev = LinearDev::new("stratis_testing_thinpool_metadata",
                                      &dm,
                                      vec![metadata_blockdev.avail_range()])
            .unwrap();

    // Clear the meta data device.  If the first block is not all zeros - the
    // stale data will cause the device to appear as an existing meta rather
    // than a new one.  Clear the entire device to be safe.  Stratis implements
    // the same approach when constructing a thin pool.
    wipe_sectors(&metadata_dev.devnode().unwrap(),
                 Sectors(0),
                 metadata_dev.size().unwrap())
            .unwrap();


    let data_dev = LinearDev::new("stratis_testing_thinpool_datadev",
                                  &dm,
                                  vec![data_blockdev.avail_range()])
            .unwrap();
    let thinpool_dev = ThinPoolDev::new("stratis_testing_thinpool",
                                        &dm,
                                        data_dev.size().unwrap(),
                                        Sectors(1024),
                                        DataBlocks(256000),
                                        metadata_dev,
                                        data_dev)
            .unwrap();
    let thin_dev = ThinDev::new("stratis_testing_thindev",
                                &dm,
                                &thinpool_dev,
                                ThinDevId::new_u64(7).expect("7 is small enough"),
                                Sectors(300000))
            .unwrap();

    create_fs(&thin_dev.devnode().unwrap()).unwrap();

    let tmp_dir = TempDir::new("stratis_testing").unwrap();
    mount_fs(&thin_dev.devnode().unwrap(), tmp_dir.path()).unwrap();
    for i in 0..100 {
        let file_path = tmp_dir.path().join(format!("stratis_test{}.txt", i));
        writeln!(&OpenOptions::new()
                      .create(true)
                      .write(true)
                      .open(file_path)
                      .unwrap(),
                 "data")
                .unwrap();
    }
    // The -d (detach-loop) is passed for both loopback and real devs,
    // it helps with loopback devs and does no harm for real devs.
    unmount_fs(tmp_dir.path(), &["-d"]).unwrap();
    thin_dev.teardown(&dm).unwrap();
    thinpool_dev.teardown(&dm).unwrap();
}

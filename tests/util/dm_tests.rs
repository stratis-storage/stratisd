// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Test the functionality of device mapper devices.


extern crate devicemapper;
extern crate libstratis;
extern crate nix;
extern crate tempdir;
extern crate uuid;

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use self::nix::mount::{MsFlags, MNT_DETACH, mount, umount2};
use self::tempdir::TempDir;
use self::uuid::Uuid;

use self::devicemapper::Bytes;
use self::devicemapper::consts::IEC;
use self::devicemapper::{DmName, DM};
use self::devicemapper::LinearDev;
use self::devicemapper::Segment;
use self::devicemapper::{DataBlocks, Sectors};
use self::devicemapper::{DmDevice, ThinDev, ThinDevId, ThinPoolDev};

use libstratis::engine::strat_engine::blockdevmgr::{BlockDevMgr, initialize, map_to_dm};
use libstratis::engine::strat_engine::device::{blkdev_size, resolve_devices, wipe_sectors};
use libstratis::engine::strat_engine::filesystem::create_fs;
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
    let total_blockdev_size: Sectors = initialized.iter().map(|i| i.avail_range().1).sum();

    let segments = initialized
        .iter()
        .map(|block_dev| {
                 let (start, length) = block_dev.avail_range();
                 Segment::new(*block_dev.device(), start, length)
             })
        .collect::<Vec<_>>();

    let dm = DM::new().unwrap();
    let lineardev = LinearDev::new(DmName::new("stratis_testing_linear").expect("valid format"),
                                   &dm,
                                   segments)
            .unwrap();
    let lineardev_size = blkdev_size(&OpenOptions::new()
                                          .read(true)
                                          .open(lineardev.devnode())
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

    let mut bd_mgr = BlockDevMgr::new(initialized);

    let dm = DM::new().unwrap();

    let meta_segs = bd_mgr
        .alloc_space(Bytes(16 * IEC::Mi).sectors())
        .unwrap();
    let metadata_dev =
        LinearDev::new(DmName::new("stratis_testing_thinpool_metadata").expect("valid format"),
                       &dm,
                       map_to_dm(&meta_segs))
                .unwrap();

    // Clear the meta data device.  If the first block is not all zeros - the
    // stale data will cause the device to appear as an existing meta rather
    // than a new one.  Clear the entire device to be safe.  Stratis implements
    // the same approach when constructing a thin pool.
    wipe_sectors(&metadata_dev.devnode(), Sectors(0), metadata_dev.size()).unwrap();

    let data_segs = bd_mgr.alloc_space(Bytes(IEC::Gi).sectors()).unwrap();
    let data_dev =
        LinearDev::new(DmName::new("stratis_testing_thinpool_datadev").expect("valid format"),
                       &dm,
                       map_to_dm(&data_segs))
                .unwrap();
    let thinpool_dev =
        ThinPoolDev::new(DmName::new("stratis_testing_thinpool").expect("valid format"),
                         &dm,
                         Sectors(1024),
                         DataBlocks(256000),
                         metadata_dev,
                         data_dev)
                .unwrap();
    let thin_dev = ThinDev::new(DmName::new("stratis_testing_thindev").expect("valid format"),
                                &dm,
                                &thinpool_dev,
                                ThinDevId::new_u64(7).expect("7 is small enough"),
                                Sectors(300000))
            .unwrap();

    create_fs(&thin_dev.devnode()).unwrap();

    let tmp_dir = TempDir::new("stratis_testing").unwrap();
    mount(Some(&thin_dev.devnode()),
          tmp_dir.path(),
          Some("xfs"),
          MsFlags::empty(),
          None as Option<&str>)
            .unwrap();
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
    // The MNT_DETACH flags is passed for both loopback and real devs,
    // it helps with loopback devs and does no harm for real devs.
    umount2(tmp_dir.path(), MNT_DETACH).unwrap();
    thin_dev.teardown(&dm).unwrap();
    thinpool_dev.teardown(&dm).unwrap();
}

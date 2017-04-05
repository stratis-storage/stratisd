// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate devicemapper;
extern crate libstratis;
extern crate loopdev;
extern crate tempdir;
extern crate time;
extern crate uuid;

use std::u8;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use devicemapper::DM;
use devicemapper::types::{Bytes, DataBlocks, Sectors};
use loopdev::{LoopControl, LoopDevice};
use tempdir::TempDir;
use uuid::Uuid;

use libstratis::consts::IEC;
use libstratis::engine::Engine;
use libstratis::engine::strat_engine::StratEngine;
use libstratis::engine::strat_engine::blockdev::{blkdev_size, initialize, resolve_devices,
                                                 wipe_sectors, BlockDev};
use libstratis::engine::strat_engine::engine::DevOwnership;
use libstratis::engine::strat_engine::lineardev::LinearDev;
use libstratis::engine::strat_engine::metadata::{StaticHeader, MIN_MDA_SECTORS};
use libstratis::engine::strat_engine::thindev::ThinDev;
use libstratis::engine::strat_engine::thinpooldev::ThinPoolDev;


/// Create a backing store from a path.
fn make_device(path: &Path) -> () {
    OpenOptions::new().read(true).write(true).create(true).open(path).unwrap();
    wipe_sectors(path, Sectors(0), Bytes(IEC::Gi as u64).sectors()).unwrap();
}

/// Setup count loop backed devices in tempdir.
fn setup_loopbacked_devices(count: u8, dir: &TempDir) -> Vec<LoopDevice> {
    let lc = LoopControl::open().unwrap();
    let mut loop_devices = Vec::new();
    for index in 0..count {
        let subdir = TempDir::new_in(dir, &index.to_string()).unwrap();
        let tmppath = subdir.path().join("store");
        make_device(&tmppath);
        let ld = lc.next_free().unwrap();
        ld.attach(tmppath, 0).unwrap();
        loop_devices.push(ld);
    }
    loop_devices
}


/// Set up count loopbacked devices.
/// Then, run the designated test.
pub fn test_with_spec<F>(count: u8, test: F) -> ()
    where F: Fn(&[&Path]) -> ()
{
    let tmpdir = TempDir::new("stratis").unwrap();
    let loop_devices: Vec<LoopDevice> = setup_loopbacked_devices(count, &tmpdir);
    let device_paths: Vec<PathBuf> = loop_devices.iter().map(|x| x.get_path().unwrap()).collect();
    let device_paths: Vec<&Path> = device_paths.iter().map(|x| x.as_path()).collect();

    test(&device_paths);

    for dev in loop_devices {
        dev.detach().unwrap();
    }
}


/// Verify that it is impossible to steal blockdevs from another Stratis
/// pool.
#[test]
pub fn test_force_flag_stratis() {

    /// 1. Initialize devices with uuid.
    /// 2. Initializing again with different uuid must fail.
    /// 3. Initializing again with same uuid must fail, because all the
    /// devices already belong.
    /// 4. Initializing again with different uuid and force = true also fails.
    fn property(paths: &[&Path]) -> () {
        let unique_devices = resolve_devices(&paths).unwrap();

        let uuid = Uuid::new_v4();
        let uuid2 = Uuid::new_v4();

        initialize(&uuid, unique_devices.clone(), MIN_MDA_SECTORS, false).unwrap();
        assert!(initialize(&uuid2, unique_devices.clone(), MIN_MDA_SECTORS, false).is_err());

        // FIXME: once requirement that number of devices added be at least 2 is removed
        // this should succeed.
        assert!(initialize(&uuid, unique_devices.clone(), MIN_MDA_SECTORS, false).is_err());

        // FIXME: this should succeed, but currently it fails, to be extra safe.
        // See: https://github.com/stratis-storage/stratisd/pull/292
        assert!(initialize(&uuid2, unique_devices.clone(), MIN_MDA_SECTORS, true).is_err());
    }

    test_with_spec(2, property);
    test_with_spec(3, property);
}


/// Verify that the sum of the lengths of the available range of the
/// blockdevs in the linear device equals the size of the linear device.
#[test]
pub fn test_linear_device() {
    fn property(paths: &[&Path]) -> () {
        let unique_devices = resolve_devices(&paths).unwrap();
        let initialized = initialize(&Uuid::new_v4(),
                                     unique_devices.clone(),
                                     MIN_MDA_SECTORS,
                                     false)
            .unwrap();
        let total_blockdev_size = initialized.iter().fold(Sectors(0), |s, i| s + i.avail_range().1);

        let initialized_refs: Vec<&BlockDev> = initialized.iter().collect();
        let device_name = "stratis_testing_linear";
        let dm = DM::new().unwrap();
        let lineardev = LinearDev::new(&device_name, &dm, &initialized_refs).unwrap();

        let mut linear_dev_path = PathBuf::from("/dev/mapper");
        linear_dev_path.push(device_name);

        let lineardev_size = blkdev_size(&File::open(linear_dev_path).unwrap()).unwrap();
        assert!(total_blockdev_size.bytes() == lineardev_size);
        lineardev.teardown(&dm).unwrap();
    }

    test_with_spec(2, property);
    test_with_spec(3, property);
}


#[test]
/// Verify no errors when writing files to a mounted filesystem on a thin
/// device.
pub fn test_thinpool_device() {
    fn property(paths: &[&Path]) -> () {
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
                                          &vec![metadata_blockdev])
            .unwrap();
        let data_dev = LinearDev::new("stratis_testing_thinpool_datadev",
                                      &dm,
                                      &vec![data_blockdev])
            .unwrap();
        let thinpool_dev = ThinPoolDev::new("stratis_testing_thinpool",
                                            &dm,
                                            data_dev.size().unwrap().sectors(),
                                            Sectors(1024),
                                            DataBlocks(256000),
                                            metadata_dev,
                                            data_dev)
            .unwrap();
        let thin_dev = ThinDev::new("stratis_testing_thindev",
                                    &dm,
                                    &thinpool_dev,
                                    7,
                                    Sectors(300000))
            .unwrap();

        thin_dev.create_fs().unwrap();

        let tmp_dir = TempDir::new("stratis_testing").unwrap();
        thin_dev.mount_fs(tmp_dir.path()).unwrap();
        thin_dev.unmount_fs(tmp_dir.path()).unwrap();
        for i in 0..100 {
            let file_path = tmp_dir.path().join(format!("stratis_test{}.txt", i));
            writeln!(&File::create(file_path).unwrap(), "data").unwrap();
        }
        thin_dev.teardown(&dm).unwrap();
        thinpool_dev.teardown(&dm).unwrap();
    }

    test_with_spec(3, property);
}


#[test]
/// Test that creating a pool claims all blockdevs and that destroying a pool
/// releases all blockdevs.
pub fn test_pool_blockdevs() {
    fn property(paths: &[&Path]) -> () {
        let mut engine = StratEngine::new();
        let (uuid, blockdevs) = engine.create_pool("test_pool", paths, None, true).unwrap();
        assert!(blockdevs.iter().all(|path| {
            StaticHeader::determine_ownership(&mut OpenOptions::new()
                    .read(true)
                    .open(path)
                    .unwrap())
                .unwrap() == DevOwnership::Ours(uuid)
        }));
        engine.destroy_pool(&uuid).unwrap();
        assert!(blockdevs.iter().all(|path| {
            StaticHeader::determine_ownership(&mut OpenOptions::new()
                    .read(true)
                    .open(path)
                    .unwrap())
                .unwrap() == DevOwnership::Unowned
        }));
    }

    test_with_spec(3, property);
}

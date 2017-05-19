// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate devicemapper;
extern crate libstratis;
extern crate rand;
extern crate tempdir;
extern crate time;
extern crate uuid;

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use self::devicemapper::DM;
use self::devicemapper::consts::SECTOR_SIZE;
use self::devicemapper::LinearDev;
use self::devicemapper::Segment;
use self::devicemapper::{DataBlocks, Sectors};
use self::devicemapper::ThinDev;
use self::devicemapper::ThinPoolDev;

use self::tempdir::TempDir;
use self::uuid::Uuid;

use libstratis::engine::{Engine, EngineError, ErrorEnum};
use libstratis::engine::strat_engine::StratEngine;
use libstratis::engine::strat_engine::blockdevmgr::{initialize, resolve_devices};
use libstratis::engine::strat_engine::device::{blkdev_size, write_sectors};
use libstratis::engine::strat_engine::engine::DevOwnership;
use libstratis::engine::strat_engine::filesystem::{create_fs, mount_fs, unmount_fs};
use libstratis::engine::strat_engine::metadata::{StaticHeader, BDA_STATIC_HDR_SECTORS,
                                                 MIN_MDA_SECTORS};
use libstratis::engine::strat_engine::setup::{find_all, get_pool_metadata};


/// Dirty sectors where specified, with 1s.
fn dirty_sectors(path: &Path, offset: Sectors, length: Sectors) {
    write_sectors(path, offset, length, &[1u8; SECTOR_SIZE]).unwrap();
}

/// Verify that it is impossible to initialize a set of disks of which
/// even one is dirty, i.e, has some data written within BDA_STATIC_HDR_SECTORS
/// of start of disk. Choose the dirty disk randomly. This means that even
/// if our code is broken with respect to this property, this test might
/// sometimes succeed.
/// FIXME: Consider enriching device specs so that this test will fail
/// consistently.
/// Verify that force flag allows all dirty disks to be initialized.
pub fn test_force_flag_dirty(paths: &[&Path]) -> () {

    let index = rand::random::<u8>() as usize % paths.len();
    dirty_sectors(paths[index],
                  Sectors(index as u64 % *BDA_STATIC_HDR_SECTORS),
                  Sectors(1));

    let unique_devices = resolve_devices(&paths).unwrap();

    let uuid = Uuid::new_v4();
    assert!(initialize(&uuid, unique_devices.clone(), MIN_MDA_SECTORS, false).is_err());
    assert!(paths
                .iter()
                .enumerate()
                .all(|(i, path)| {
        StaticHeader::determine_ownership(&mut OpenOptions::new().read(true).open(path).unwrap())
            .unwrap() ==
        if i == index {
            DevOwnership::Theirs
        } else {
            DevOwnership::Unowned
        }
    }));

    assert!(initialize(&uuid, unique_devices.clone(), MIN_MDA_SECTORS, true).is_ok());
    assert!(paths
                .iter()
                .all(|path| {
                         StaticHeader::determine_ownership(&mut OpenOptions::new()
                                                                    .read(true)
                                                                    .open(path)
                                                                    .unwrap())
                                 .unwrap() == DevOwnership::Ours(uuid)
                     }));
}


/// Verify that it is impossible to steal blockdevs from another Stratis
/// pool.
/// 1. Initialize devices with pool uuid.
/// 2. Initializing again with different uuid must fail.
/// 3. Initializing again with same pool uuid must succeed, because all the
/// devices already belong so there's nothing to do.
/// 4. Initializing again with different uuid and force = true also fails.
pub fn test_force_flag_stratis(paths: &[&Path]) -> () {
    let unique_devices = resolve_devices(&paths).unwrap();

    let uuid = Uuid::new_v4();
    let uuid2 = Uuid::new_v4();

    initialize(&uuid, unique_devices.clone(), MIN_MDA_SECTORS, false).unwrap();
    assert!(initialize(&uuid2, unique_devices.clone(), MIN_MDA_SECTORS, false).is_err());

    assert!(initialize(&uuid, unique_devices.clone(), MIN_MDA_SECTORS, false).is_ok());

    // FIXME: this should succeed, but currently it fails, to be extra safe.
    // See: https://github.com/stratis-storage/stratisd/pull/292
    assert!(initialize(&uuid2, unique_devices.clone(), MIN_MDA_SECTORS, true).is_err());
}

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
        .fold(Sectors(0), |s, i| s + i.avail_range_segment().range().1);

    let segments = initialized
        .iter()
        .map(|block_dev| block_dev.avail_range_segment())
        .collect::<Vec<Segment>>();

    let device_name = "stratis_testing_linear";
    let dm = DM::new().unwrap();
    let lineardev = LinearDev::new(&device_name, &dm, segments).unwrap();

    let mut linear_dev_path = PathBuf::from("/dev/mapper");
    linear_dev_path.push(device_name);

    let lineardev_size = blkdev_size(&File::open(linear_dev_path).unwrap()).unwrap();
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
                                      vec![metadata_blockdev.avail_range_segment()])
            .unwrap();
    let data_dev = LinearDev::new("stratis_testing_thinpool_datadev",
                                  &dm,
                                  vec![data_blockdev.avail_range_segment()])
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

    create_fs(&thin_dev.devnode().unwrap()).unwrap();

    let tmp_dir = TempDir::new("stratis_testing").unwrap();
    mount_fs(&thin_dev.devnode().unwrap(), tmp_dir.path()).unwrap();
    unmount_fs(tmp_dir.path()).unwrap();
    for i in 0..100 {
        let file_path = tmp_dir.path().join(format!("stratis_test{}.txt", i));
        writeln!(&File::create(file_path).unwrap(), "data").unwrap();
    }
    thin_dev.teardown(&dm).unwrap();
    thinpool_dev.teardown(&dm).unwrap();
}


/// Test that creating a pool claims all blockdevs and that destroying a pool
/// releases all blockdevs.
pub fn test_pool_blockdevs(paths: &[&Path]) -> () {
    let mut engine = StratEngine::new();
    let (uuid, blockdevs) = engine
        .create_pool("test_pool", paths, None, true)
        .unwrap();
    assert!(blockdevs
                .iter()
                .all(|path| {
                         StaticHeader::determine_ownership(&mut OpenOptions::new()
                                                                    .read(true)
                                                                    .open(path)
                                                                    .unwrap())
                                 .unwrap() == DevOwnership::Ours(uuid)
                     }));
    engine.destroy_pool(&uuid).unwrap();
    assert!(blockdevs
                .iter()
                .all(|path| {
                         StaticHeader::determine_ownership(&mut OpenOptions::new()
                                                                    .read(true)
                                                                    .open(path)
                                                                    .unwrap())
                                 .unwrap() == DevOwnership::Unowned
                     }));
}


/// Verify that tearing down an engine doesn't fail if no filesystems on it.
pub fn test_teardown(paths: &[&Path]) -> () {
    let mut engine = StratEngine::new();
    engine
        .create_pool("test_pool", paths, None, true)
        .unwrap();
    assert!(engine.teardown().is_ok())
}

/// Verify that find_all function locates and assigns pools appropriately.
/// 1. Split available paths into 2 discrete sets.
/// 2. Initialize the block devices in the first set with a pool uuid.
/// 3. Run find_all() and verify that it has found the initialized devices
/// and no others.
/// 4. Initialize the block devices in the second set with a different pool
/// uuid.
/// 5. Run find_all() again and verify that both sets of devices are found.
/// 6. Verify that get_pool_metadata() return an error. initialize() only
/// initializes block devices, it does not write metadata.
pub fn test_setup(paths: &[&Path]) -> () {
    assert!(paths.len() > 2);

    let (paths1, paths2) = paths.split_at(2);

    let unique_devices = resolve_devices(paths1).unwrap();
    let uuid1 = Uuid::new_v4();
    initialize(&uuid1, unique_devices, MIN_MDA_SECTORS, false).unwrap();

    let pools = find_all().unwrap();
    assert!(pools.len() == 1);
    assert!(pools.contains_key(&uuid1));
    let devices = pools.get(&uuid1).expect("pools.contains_key() was true");
    assert!(devices.len() == paths1.len());

    let unique_devices = resolve_devices(paths2).unwrap();
    let uuid2 = Uuid::new_v4();
    initialize(&uuid2, unique_devices, MIN_MDA_SECTORS, false).unwrap();

    let pools = find_all().unwrap();
    assert!(pools.len() == 2);

    assert!(pools.contains_key(&uuid1));
    let devices1 = pools.get(&uuid1).expect("pools.contains_key() was true");
    assert!(devices1.len() == paths1.len());

    assert!(pools.contains_key(&uuid2));
    let devices2 = pools.get(&uuid2).expect("pools.contains_key() was true");
    assert!(devices2.len() == paths2.len());

    assert!(get_pool_metadata(&pools).is_err());
}

/// Verify that a pool with no devices does not have the minimum amount of
/// space required.
pub fn test_empty_pool(paths: &[&Path]) -> () {
    assert!(paths.len() == 0);
    let mut engine = StratEngine::new();
    assert!(match engine
                      .create_pool("test_pool", paths, None, true)
                      .unwrap_err() {
                EngineError::Engine(ErrorEnum::Invalid, _) => true,
                _ => false,
            });
}

/// Verify that metadata can be read from pools.
/// 1. Split paths into two separate sets.
/// 2. Create a pool from the first set.
/// 3. Use find_all() to get the devices in the pool.
/// 4. Use get_pool_metadata to find metadata for all pools.
/// 5. Verify that metadata name is correct.
/// 6. Create a second pool and repeat.
/// 7. Teardown the engine and repeat.
pub fn test_basic_metadata(paths: &[&Path]) {
    assert!(paths.len() > 2);

    let (paths1, paths2) = paths.split_at(2);

    let mut engine = StratEngine::new();

    let name1 = "name1";
    let (uuid1, _) = engine.create_pool(&name1, paths1, None, false).unwrap();

    let pools = find_all().unwrap();
    let metadata = get_pool_metadata(&pools).unwrap();
    assert!(metadata.len() == 1);
    assert!(metadata.get(&uuid1).unwrap().name == name1);

    let name2 = "name2";
    let (uuid2, _) = engine.create_pool(&name2, paths2, None, false).unwrap();

    let pools = find_all().unwrap();
    let metadata = get_pool_metadata(&pools).unwrap();
    assert!(metadata.len() == 2);
    assert!(metadata.get(&uuid1).unwrap().name == name1);
    assert!(metadata.get(&uuid2).unwrap().name == name2);

    engine.teardown().unwrap();
    let pools = find_all().unwrap();
    let metadata = get_pool_metadata(&pools).unwrap();
    assert!(metadata.len() == 2);
    assert!(metadata.get(&uuid1).unwrap().name == name1);
    assert!(metadata.get(&uuid2).unwrap().name == name2);
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Test the functionality of stratis pools.
extern crate devicemapper;
extern crate env_logger;
extern crate uuid;
extern crate nix;
extern crate tempdir;

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use self::nix::mount::{mount, MsFlags, umount};
use self::tempdir::TempDir;

use self::devicemapper::{Device, DmName, DM, SECTOR_SIZE, Sectors, ThinDev};

use libstratis::engine::{Engine, Pool};
use libstratis::engine::engine::HasUuid;
use libstratis::engine::strat_engine::StratEngine;
use libstratis::engine::strat_engine::device::resolve_devices;
use libstratis::engine::strat_engine::pool::{DATA_BLOCK_SIZE, DATA_LOWATER, INITIAL_DATA_SIZE,
                                             StratPool};
use libstratis::engine::types::{Redundancy, RenameAction};

/// Verify that the physical space allocated to a pool is expanded when
/// the number of sectors written to a thin-dev in the pool exceeds the
/// INITIAL_DATA_SIZE.  If we are able to write more sectors to the filesystem
/// than are initially allocated to the pool, the pool must have been expanded.
pub fn test_thinpool_expand(paths: &[&Path]) -> () {
    let (mut pool, _) = StratPool::initialize("stratis_test_pool",
                                              &DM::new().unwrap(),
                                              paths,
                                              Redundancy::NONE,
                                              true)
            .unwrap();

    let &(_, fs_uuid) = pool.create_filesystems(&[("stratis_test_filesystem", None)])
        .unwrap()
        .first()
        .unwrap();

    let devnode = pool.get_filesystem(fs_uuid).unwrap().devnode();
    // Braces to ensure f is closed before destroy
    {
        let mut f = OpenOptions::new().write(true).open(devnode).unwrap();
        // Write 1 more sector than is initially allocated to a pool
        let write_size = *INITIAL_DATA_SIZE * DATA_BLOCK_SIZE + Sectors(1);
        let buf = &[1u8; SECTOR_SIZE];
        for i in 0..*write_size {
            f.write_all(buf).unwrap();
            // Simulate handling a DM event by running a pool check at the point where
            // the amount of free space in pool has decreased to the DATA_LOWATER value.
            // TODO: Actually handle DM events and possibly call extend() directly,
            // depending on the specificity of the events.
            if i == *(*(INITIAL_DATA_SIZE - DATA_LOWATER) * DATA_BLOCK_SIZE) {
                pool.check().unwrap();
            }
        }
    }
    pool.destroy_filesystems(&[fs_uuid]).unwrap();
    pool.teardown().unwrap();
}

/// Verify a snapshot has the same files and same contents as the origin.
pub fn test_filesystem_snapshot(paths: &[&Path]) {
    let dm = DM::new().unwrap();
    let (mut pool, _) =
        StratPool::initialize("stratis_test_pool", &dm, paths, Redundancy::NONE, true).unwrap();
    let &(_, fs_uuid) = pool.create_filesystems(&[("stratis_test_filesystem", None)])
        .unwrap()
        .first()
        .unwrap();
    let write_buf = &[8u8; SECTOR_SIZE];
    let file_count = 10;
    let source_tmp_dir = TempDir::new("stratis_testing").unwrap();
    {
        // to allow mutable borrow of pool
        let filesystem = pool.get_filesystem(fs_uuid).unwrap();
        mount(Some(&filesystem.devnode()),
              source_tmp_dir.path(),
              Some("xfs"),
              MsFlags::empty(),
              None as Option<&str>)
                .unwrap();
        for i in 0..file_count {
            let file_path = source_tmp_dir
                .path()
                .join(format!("stratis_test{}.txt", i));
            let mut f = OpenOptions::new()
                .create(true)
                .write(true)
                .open(file_path)
                .unwrap();
            f.write_all(write_buf).unwrap();
            f.flush().unwrap();

        }

    }
    // Run a check to expand the pool. The space initially allocated
    // to a pool is close to consumed by the filesystem and few files
    // written above. If we attempt to update the UUID on the snapshot
    // without expanding the pool, the pool will go into out-of-data-space
    // (queue IO) mode, causing the test to fail. Calling pool.check() will
    // compare the data space used by the pool to the data space allocated
    // and expand when there is less than DATA_LOWATER space remaining.
    // TODO: Make use of a way (not as yet existing) to explicitly extend
    // pool to the necessary size
    pool.check().unwrap();

    let snapshot_uuid = pool.snapshot_filesystem(fs_uuid).unwrap();
    let mut read_buf = [0u8; SECTOR_SIZE];
    let snapshot_tmp_dir = TempDir::new("stratis_testing").unwrap();
    {
        let snapshot_filesystem = pool.get_filesystem(snapshot_uuid).unwrap();
        mount(Some(&snapshot_filesystem.devnode()),
              snapshot_tmp_dir.path(),
              Some("xfs"),
              MsFlags::empty(),
              None as Option<&str>)
                .unwrap();
        for i in 0..file_count {
            let file_path = snapshot_tmp_dir
                .path()
                .join(format!("stratis_test{}.txt", i));
            let mut f = OpenOptions::new().read(true).open(file_path).unwrap();
            f.read(&mut read_buf).unwrap();
            assert!(read_buf[0..SECTOR_SIZE] == write_buf[0..SECTOR_SIZE]);
        }
    }
    umount(source_tmp_dir.path()).unwrap();
    umount(snapshot_tmp_dir.path()).unwrap();
    pool.destroy_filesystems(&[fs_uuid, snapshot_uuid])
        .unwrap();
    pool.teardown().unwrap();
}

/// Verify that a filesystem rename causes the filesystem metadata to be
/// updated.
pub fn test_filesystem_rename(paths: &[&Path]) {
    let mut engine = StratEngine::initialize().unwrap();

    let name1 = "name1";
    let name2 = "name2";
    let (uuid1, _) = engine.create_pool(&name1, paths, None, false).unwrap();
    let fs_uuid = {
        let pool = engine.get_mut_pool(uuid1).unwrap();
        let &(fs_name, fs_uuid) = pool.create_filesystems(&[(name1, None)])
            .unwrap()
            .first()
            .unwrap();

        assert_eq!(name1, fs_name);

        let action = pool.rename_filesystem(fs_uuid, name2).unwrap();
        assert_eq!(action, RenameAction::Renamed);
        fs_uuid
    };
    engine.teardown().unwrap();

    let engine = StratEngine::initialize().unwrap();
    let filesystem_name: String = engine
        .get_pool(uuid1)
        .unwrap()
        .get_filesystem(fs_uuid)
        .unwrap()
        .name()
        .into();
    assert_eq!(filesystem_name, name2);

    engine.teardown().unwrap();
}

/// Verify that destroy_filesystems actually deallocates the space
/// from the thinpool, by attempting to reinstantiate it using the
/// same thin id and verifying that it fails.
pub fn test_thinpool_thindev_destroy(paths: &[&Path]) -> () {
    let (mut pool, _) = StratPool::initialize("stratis_test_pool",
                                              &DM::new().unwrap(),
                                              paths,
                                              Redundancy::NONE,
                                              true)
            .unwrap();
    let &(_, fs_uuid) = pool.create_filesystems(&[("stratis_test_filesystem", None)])
        .unwrap()
        .first()
        .unwrap();

    let fs_id = pool.get_mut_strat_filesystem(fs_uuid)
        .unwrap()
        .thin_id();

    pool.destroy_filesystems(&[fs_uuid]).unwrap();

    let pool_uuid = pool.uuid();

    // Try to setup a thindev that has been destroyed
    let dm = DM::new().unwrap();
    let thindev = ThinDev::setup(&dm,
                                 DmName::new("stratis_test_thin_dev").expect("valid format"),
                                 None,
                                 pool.thinpooldev(),
                                 fs_id,
                                 Sectors(128u64));
    assert!(thindev.is_err());
    pool.teardown().unwrap();

    // Check that destroyed fs is not present in MDV. If the record
    // had been left on the MDV that didn't match a thin_id in the
    // thinpool, ::setup() will fail.
    let paths2: HashMap<Device, PathBuf> = resolve_devices(paths)
        .unwrap()
        .into_iter()
        .map(|(d, p)| (d, p.to_owned()))
        .collect();
    let pool = StratPool::setup(pool_uuid, &paths2).unwrap();

    // This also should never happen, given the previous two parts of
    // this test.
    assert!(pool.get_filesystem(fs_uuid).is_none());
    pool.teardown().unwrap();
}

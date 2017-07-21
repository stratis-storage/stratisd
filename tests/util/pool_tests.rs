// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Test the functionality of stratis pools.
extern crate devicemapper;
extern crate env_logger;

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use self::devicemapper::DM;
use self::devicemapper::Sectors;
use self::devicemapper::consts::SECTOR_SIZE;

use libstratis::engine::{Engine, Pool};
use libstratis::engine::strat_engine::StratEngine;
use libstratis::engine::strat_engine::pool::{DATA_BLOCK_SIZE, DATA_LOWATER, INITIAL_DATA_SIZE,
                                             StratPool};
use libstratis::engine::types::{Redundancy, RenameAction};

/// Verify that a the physical space allocated to a pool is expanded when
/// the nuber of sectors written to a thin-dev in the pool exceeds the
/// INITIAL_DATA_SIZE.  If we are able to write more sectors to the filesystem
/// than are initially allocated to the pool, the pool must have been expanded.
pub fn test_thinpool_expand(paths: &[&Path]) -> () {
    let (mut pool, _) = StratPool::initialize("stratis_test_pool",
                                              &DM::new().unwrap(),
                                              paths,
                                              Redundancy::NONE,
                                              true)
            .unwrap();
    let &(_, fs_uuid) = pool.create_filesystems(&vec!["stratis_test_filesystem"], None)
        .unwrap()
        .first()
        .unwrap();

    let devnode = pool.get_filesystem(&fs_uuid)
        .unwrap()
        .devnode()
        .unwrap();
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
                pool.check();
            }
        }
    }
    pool.destroy_filesystems(&[&fs_uuid]).unwrap();
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
        let mut pool = engine.get_pool(&uuid1).unwrap();
        let &(fs_name, fs_uuid) = pool.create_filesystems(&[name1], None)
            .unwrap()
            .first()
            .unwrap();

        assert_eq!(name1, fs_name);

        let action = pool.rename_filesystem(&fs_uuid, name2).unwrap();
        assert_eq!(action, RenameAction::Renamed);
        fs_uuid
    };
    engine.teardown().unwrap();

    let mut engine = StratEngine::initialize().unwrap();
    let filesystem_name: String = engine
        .get_pool(&uuid1)
        .unwrap()
        .get_filesystem(&fs_uuid)
        .unwrap()
        .name()
        .into();
    assert_eq!(filesystem_name, name2);

    engine.teardown().unwrap();
}

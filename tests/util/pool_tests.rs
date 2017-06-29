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

use libstratis::engine::Pool;
use libstratis::engine::strat_engine::pool::{DATA_BLOCK_SIZE, DATA_LOWATER, INITIAL_DATA_SIZE,
                                             StratPool};
use libstratis::engine::strat_engine::StratEngine;
use libstratis::engine::types::Redundancy;

/// Verify that a the physical space allocated to a pool is expanded when
/// the nuber of sectors written to a thin-dev in the pool exceeds the
/// INITIAL_DATA_SIZE.  If we are able to write more sectors to the filesystem
/// than are initially allocated to the pool, the pool must have been expanded.
pub fn test_thinpool_expand(paths: &[&Path]) -> () {
    StratEngine::initialize().unwrap();
    let mut pool = StratPool::initialize("stratis_test_pool",
                                         &DM::new().unwrap(),
                                         paths,
                                         Redundancy::NONE,
                                         true)
            .unwrap();
    let &(_, fs_uuid) = pool.create_filesystems(&vec!["stratis_test_filesystem"])
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
        let write_size = INITIAL_DATA_SIZE + Sectors(1);
        let buf = &[1u8; SECTOR_SIZE];
        for i in 0..*write_size {
            f.write_all(buf).unwrap();
            // Simulate handling a DM event by running a pool check at the point where
            // the amount of free space in pool has decreased to the DATA_LOWATER value.
            // TODO: Actually handle DM events and possibly call extend() directly,
            // depending on the specificity of the events.
            if i == *(INITIAL_DATA_SIZE - Sectors(*DATA_LOWATER * *DATA_BLOCK_SIZE)) {
                pool.check();
            }
        }
    }
    pool.destroy_filesystems(&[&fs_uuid]).unwrap();
    pool.teardown().unwrap();
}

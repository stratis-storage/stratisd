// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Tests that focus on lower-level blockdev functionality.


extern crate devicemapper;
extern crate libstratis;
extern crate rand;
extern crate uuid;

use std::fs::OpenOptions;
use std::path::Path;

use self::uuid::Uuid;

use self::devicemapper::Sectors;
use self::devicemapper::consts::SECTOR_SIZE;

use libstratis::engine::Engine;
use libstratis::engine::strat_engine::blockdevmgr::{BlockDevMgr, initialize};
use libstratis::engine::strat_engine::device::{resolve_devices, write_sectors};
use libstratis::engine::strat_engine::engine::DevOwnership;
use libstratis::engine::strat_engine::metadata::{StaticHeader, BDA_STATIC_HDR_SECTORS,
                                                 MIN_MDA_SECTORS};
use libstratis::engine::strat_engine::StratEngine;

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


/// Test that creating a pool claims all blockdevs and that destroying a pool
/// releases all blockdevs.
pub fn test_pool_blockdevs(paths: &[&Path]) -> () {
    let mut engine = StratEngine::initialize().unwrap();
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

/// Verify that initially, current_capacity() - metadata_size() = avail_space()
/// After 2 Sectors have been allocated, that amount must also be included in
/// balance.
pub fn test_blockdevmgr_used(paths: &[&Path]) -> () {
    let uuid = Uuid::new_v4();
    let mut mgr = BlockDevMgr::initialize(&uuid, paths, MIN_MDA_SECTORS, false).unwrap();
    assert_eq!(mgr.avail_space() + mgr.metadata_size(),
               mgr.current_capacity());

    let allocated = Sectors(2);
    mgr.alloc_space(allocated).unwrap();
    assert_eq!(mgr.avail_space() + allocated + mgr.metadata_size(),
               mgr.current_capacity());
}

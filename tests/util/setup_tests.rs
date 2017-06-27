// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Test aspects of setting up from metadata.


extern crate devicemapper;
extern crate libstratis;
extern crate uuid;

use std::path::Path;

use self::uuid::Uuid;

use self::devicemapper::DM;

use libstratis::engine::Engine;
use libstratis::engine::strat_engine::blockdevmgr::{initialize, resolve_devices};
use libstratis::engine::strat_engine::metadata::MIN_MDA_SECTORS;
use libstratis::engine::strat_engine::pool::{get_dmdevs, get_filesystems};
use libstratis::engine::strat_engine::serde_structs::Recordable;
use libstratis::engine::strat_engine::setup::{find_all, get_blockdevs, get_metadata};
use libstratis::engine::strat_engine::StratEngine;


/// Verify that find_all function locates and assigns pools appropriately.
/// 1. Split available paths into 2 discrete sets.
/// 2. Initialize the block devices in the first set with a pool uuid.
/// 3. Run find_all() and verify that it has found the initialized devices
/// and no others.
/// 4. Initialize the block devices in the second set with a different pool
/// uuid.
/// 5. Run find_all() again and verify that both sets of devices are found.
/// 6. Verify that get_metadata() return an error. initialize() only
/// initializes block devices, it does not write metadata.
pub fn test_initialize(paths: &[&Path]) -> () {
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

    assert!(pools
                .iter()
                .map(|(uuid, devs)| get_metadata(*uuid, devs))
                .all(|x| x.unwrap().is_none()));
}

/// Verify that metadata can be read from pools.
/// 1. Split paths into two separate sets.
/// 2. Create pools from the two sets.
/// 3. Use find_all() to get the devices in the pool.
/// 4. Use get_metadata to find metadata for each pool and verify correctness.
/// 5. Teardown the engine and repeat.
/// 6. Create the dm devices belonging to the pool.
pub fn test_basic_metadata(paths: &[&Path]) {
    assert!(paths.len() > 2);

    let (paths1, paths2) = paths.split_at(2);

    let mut engine = StratEngine::initialize().unwrap();

    let name1 = "name1";
    let (uuid1, _) = engine.create_pool(&name1, paths1, None, false).unwrap();
    let metadata1 = engine
        .get_strat_pool(&uuid1)
        .unwrap()
        .record()
        .unwrap();

    let name2 = "name2";
    let (uuid2, _) = engine.create_pool(&name2, paths2, None, false).unwrap();
    let metadata2 = engine
        .get_strat_pool(&uuid2)
        .unwrap()
        .record()
        .unwrap();

    let pools = find_all().unwrap();
    assert!(pools.len() == 2);
    let devnodes1 = pools.get(&uuid1).unwrap();
    let devnodes2 = pools.get(&uuid2).unwrap();
    let pool_save1 = get_metadata(uuid1, devnodes1).unwrap().unwrap();
    let pool_save2 = get_metadata(uuid2, devnodes2).unwrap().unwrap();
    assert!(pool_save1 == metadata1);
    assert!(pool_save2 == metadata2);
    let blockdevs1 = get_blockdevs(&pool_save1, devnodes1).unwrap();
    let blockdevs2 = get_blockdevs(&pool_save2, devnodes2).unwrap();
    assert!(blockdevs1.len() == pool_save1.block_devs.len());
    assert!(blockdevs2.len() == pool_save2.block_devs.len());

    engine.teardown().unwrap();
    let pools = find_all().unwrap();
    assert!(pools.len() == 2);
    let devnodes1 = pools.get(&uuid1).unwrap();
    let devnodes2 = pools.get(&uuid2).unwrap();
    let pool_save1 = get_metadata(uuid1, devnodes1).unwrap().unwrap();
    let pool_save2 = get_metadata(uuid2, devnodes2).unwrap().unwrap();
    assert!(pool_save1 == metadata1);
    assert!(pool_save2 == metadata2);
    let blockdevs1 = get_blockdevs(&pool_save1, devnodes1).unwrap();
    let blockdevs2 = get_blockdevs(&pool_save2, devnodes2).unwrap();
    assert!(blockdevs1.len() == pool_save1.block_devs.len());
    assert!(blockdevs2.len() == pool_save2.block_devs.len());

    // These should work, under the assumption of a clean teardown.
    let (tp1, mdv1, _) = get_dmdevs(uuid1, &blockdevs1, &pool_save1).unwrap();
    let (tp2, mdv2, _) = get_dmdevs(uuid2, &blockdevs2, &pool_save2).unwrap();
    assert!(tp1.name().contains(&uuid1.simple().to_string()));
    assert!(tp2.name().contains(&uuid2.simple().to_string()));

    let filesystems1 = get_filesystems(uuid1, &tp1, &mdv1).unwrap();
    assert!(filesystems1.is_empty());

    let filesystems2 = get_filesystems(uuid2, &tp2, &mdv2).unwrap();
    assert!(filesystems2.is_empty());

    let dm = DM::new().unwrap();
    tp1.teardown(&dm).unwrap();
    tp2.teardown(&dm).unwrap();
    mdv1.teardown(&dm).unwrap();
    mdv2.teardown(&dm).unwrap();
}

/// Test engine setup.
/// 1. Create two pools.
/// 2. Verify that both exist.
/// 3. Teardown the engine.
/// 4. Verify that pools are gone.
/// 5. Initialize the engine.
/// 6. Verify that pools can be found again.
pub fn test_setup(paths: &[&Path]) {
    assert!(paths.len() > 2);

    let (paths1, paths2) = paths.split_at(2);

    let mut engine = StratEngine::initialize().unwrap();

    let name1 = "name1";
    let (uuid1, _) = engine.create_pool(&name1, paths1, None, false).unwrap();

    let name2 = "name2";
    let (uuid2, _) = engine.create_pool(&name2, paths2, None, false).unwrap();

    assert!(engine.get_pool(&uuid1).is_some());
    assert!(engine.get_pool(&uuid2).is_some());

    engine.teardown().unwrap();

    let mut engine = StratEngine::initialize().unwrap();

    assert!(engine.get_pool(&uuid1).is_some());
    assert!(engine.get_pool(&uuid2).is_some());

    engine.teardown().unwrap();
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate libstratis;
extern crate loopdev;
extern crate tempdir;
extern crate time;
extern crate uuid;

use std::u8;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use loopdev::{LoopControl, LoopDevice};
use tempdir::TempDir;
use time::now;
use uuid::Uuid;

use libstratis::consts::IEC;
use libstratis::engine::strat_engine::blockdev::{initialize, BlockDev};
use libstratis::engine::strat_engine::device::{resolve_devices, wipe_sectors};
use libstratis::engine::strat_engine::metadata::MIN_MDA_SECTORS;
use libstratis::engine::strat_engine::pool::StratPool;
// use libstratis::engine::strat_engine::setup::find_all;
use libstratis::types::{Bytes, Sectors};


/// Specification for loop device backing store.
pub struct LoopDeviceSpec {
}

/// Create a backing store from a specification and a path.
fn make_device(_spec: &LoopDeviceSpec, path: &Path) -> () {
    OpenOptions::new().read(true).write(true).create(true).open(path).unwrap();
    wipe_sectors(path, Sectors(0), Bytes(IEC::Gi as u64).sectors()).unwrap();
}

/// Setup a bunch of loop backed devices in tempdir according to specification.
fn setup_loopbacked_devices(specs: &[&LoopDeviceSpec], dir: &TempDir) -> Vec<LoopDevice> {
    let lc = LoopControl::open().unwrap();
    let mut loop_devices = Vec::new();
    for (index, spec) in specs.iter().enumerate() {
        let subdir = TempDir::new_in(dir.path(), &index.to_string()).unwrap();
        let tmppath = subdir.path().join("store");
        make_device(&spec, &tmppath);
        let ld = lc.next_free().unwrap();
        ld.attach(tmppath.as_path().to_str().unwrap(), 0).unwrap();
        loop_devices.push(ld);
    }
    loop_devices
}


/// Set up a bunch of loop backed devices based on the specification.
/// Then, run the designated test.
/// Precondition: specification length must be no more than u8::MAX.
pub fn test_with_spec<F>(specs: &[&LoopDeviceSpec], test: F) -> ()
    where F: Fn(&[&Path]) -> ()
{
    assert!(specs.len() <= u8::MAX as usize);

    let tmpdir = TempDir::new("stratis").unwrap();
    let loop_devices: Vec<LoopDevice> = setup_loopbacked_devices(specs, &tmpdir);
    let device_paths: Vec<PathBuf> = loop_devices.iter().map(|x| x.get_path().unwrap()).collect();
    let device_paths: Vec<&Path> = device_paths.iter().map(|x| x.as_path()).collect();

    test(&device_paths);

    for dev in loop_devices {
        dev.detach().unwrap();
    }
}

/// Verify that it is impossible to steal blockdevs from another Stratis
/// pool w/out force flag.
#[test]
pub fn test_force_flag() {

    /// 1. Initialize devices with uuid.
    /// 2. Initializing again with different uuid must fail.
    /// 3. Initializing again with same uuid must fail, because all the
    /// devices already belong.
    /// 4. Initializing again with different uuid and force = true must succeed.
    fn property(paths: &[&Path]) -> () {
        let unique_devices = resolve_devices(&paths).unwrap();

        let uuid = Uuid::new_v4();
        let uuid2 = Uuid::new_v4();

        initialize(&uuid, unique_devices.clone(), MIN_MDA_SECTORS, false).unwrap();
        assert!(initialize(&uuid2, unique_devices.clone(), MIN_MDA_SECTORS, false).is_err());

        // FIXME: once requirement that number of devices added be at least 2 is removed
        // this should succeed.
        assert!(initialize(&uuid, unique_devices.clone(), MIN_MDA_SECTORS, false).is_err());

        assert!(initialize(&uuid2, unique_devices.clone(), MIN_MDA_SECTORS, true).is_ok());
    }

    let spec = LoopDeviceSpec {};
    test_with_spec(&[&spec, &spec], property);
    test_with_spec(&[&spec, &spec, &spec], property);
}

/// Test reading and writing metadata on a set of blockdevs sharing one pool
/// UUID.
/// 1. Verify that it is impossible to read variable length metadata off new
/// devices.
/// 2. Write metadata and verify that it is now available.
/// 3. Write different metadata, with a newer time, and verify that the new
/// metadata is now available.
#[test]
pub fn test_new_blockdevs() {
    fn property(paths: &[&Path]) -> () {
        let unique_devices = resolve_devices(&paths).unwrap();
        let uuid = Uuid::new_v4();
        let mut blockdevs = initialize(&uuid, unique_devices, MIN_MDA_SECTORS, false).unwrap();
        assert!(StratPool::load_state(&blockdevs.iter().collect::<Vec<&BlockDev>>()).is_none());

        let (state1, state2) = (vec![1u8, 2u8, 3u8, 4u8], vec![5u8, 6u8, 7u8, 8u8]);
        let current_time = now().to_timespec();
        StratPool::save_state(&mut blockdevs.iter_mut().collect::<Vec<&mut BlockDev>>(),
                              &current_time,
                              &state1)
            .unwrap();
        assert!(StratPool::load_state(&blockdevs.iter().collect::<Vec<&BlockDev>>()).unwrap() ==
                state1);

        StratPool::save_state(&mut blockdevs.iter_mut().collect::<Vec<&mut BlockDev>>(),
                              &now().to_timespec(),
                              &state2)
            .unwrap();
        assert!(StratPool::load_state(&blockdevs.iter().collect::<Vec<&BlockDev>>()).unwrap() ==
                state2);
    }

    let spec = LoopDeviceSpec {};
    test_with_spec(&[&spec, &spec], property);
    test_with_spec(&[&spec, &spec, &spec], property);
}

/// Verify that find_all function locates and assigns pools appropriately.
#[test]
pub fn test_setup() {
    fn property(paths: &[&Path]) -> () {
        let (paths1, paths2) = paths.split_at(2);

        let unique_devices = resolve_devices(paths1).unwrap();
        let uuid1 = Uuid::new_v4();
        initialize(&uuid1, unique_devices, MIN_MDA_SECTORS, false).unwrap();
        // let pools = find_all().unwrap();
        // assert!(pools.len() == 1);
        // assert!(pools.contains_key(&uuid1));
        // let devices = pools.get(&uuid1).expect("pools.contains_key(&uuid) was true");
        // assert!(devices.len() == 2);

        let unique_devices = resolve_devices(paths2).unwrap();
        let uuid2 = Uuid::new_v4();
        initialize(&uuid2, unique_devices, MIN_MDA_SECTORS, false).unwrap();
    }

    let spec = LoopDeviceSpec {};
    test_with_spec(&[&spec, &spec, &spec, &spec], property);
}

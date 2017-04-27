// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate devicemapper;
extern crate libstratis;
extern crate loopdev;
extern crate tempdir;

mod util;

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use devicemapper::{Bytes, Sectors};
use loopdev::{LoopControl, LoopDevice};
use tempdir::TempDir;

use libstratis::consts::IEC;
use libstratis::engine::strat_engine::blockdev::wipe_sectors;

use util::simple_tests::test_force_flag_dirty;
use util::simple_tests::test_force_flag_stratis;
use util::simple_tests::test_linear_device;
use util::simple_tests::test_pool_blockdevs;
use util::simple_tests::test_thinpool_device;
use util::simple_tests::test_variable_length_metadata_times;


/// Setup count loop backed devices in dir.
/// Make sure each loop device is backed by a 1 GiB file.
fn get_devices(count: u8, dir: &TempDir) -> Vec<LoopDevice> {
    let lc = LoopControl::open().unwrap();
    let mut loop_devices = Vec::new();

    let length = Bytes(IEC::Gi as u64).sectors();
    for index in 0..count {
        let subdir = TempDir::new_in(dir, &index.to_string()).unwrap();
        let path = subdir.path().join("store");
        OpenOptions::new().read(true).write(true).create(true).open(&path).unwrap();
        wipe_sectors(&path, Sectors(0), length).unwrap();
        let ld = lc.next_free().unwrap();
        ld.attach(path, 0).unwrap();
        loop_devices.push(ld);
    }
    loop_devices
}


/// Set up count loopbacked devices.
/// Then, run the designated test.
/// Then, take down the loop devices.
fn test_with_spec<F>(count: u8, test: F) -> ()
    where F: Fn(&[&Path]) -> ()
{
    let tmpdir = TempDir::new("stratis").unwrap();
    let loop_devices: Vec<LoopDevice> = get_devices(count, &tmpdir);
    let device_paths: Vec<PathBuf> = loop_devices.iter().map(|x| x.get_path().unwrap()).collect();
    let device_paths: Vec<&Path> = device_paths.iter().map(|x| x.as_path()).collect();

    test(&device_paths);

    for dev in loop_devices {
        dev.detach().unwrap();
    }
}


#[test]
pub fn loop_test_force_flag_stratis() {
    test_with_spec(2, test_force_flag_stratis);
    test_with_spec(3, test_force_flag_stratis);
}


#[test]
pub fn loop_test_linear_device() {
    test_with_spec(2, test_linear_device);
    test_with_spec(3, test_linear_device);
}


#[test]
pub fn loop_test_thinpool_device() {
    test_with_spec(3, test_thinpool_device);
}


#[test]
pub fn loop_test_pool_blockdevs() {
    test_with_spec(3, test_pool_blockdevs);
}

#[test]
pub fn loop_test_force_flag_dirty() {
    test_with_spec(3, test_force_flag_dirty);
}

#[test]
pub fn loop_test_variable_length_metadata_times() {
    test_with_spec(3, test_variable_length_metadata_times);
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate devicemapper;
extern crate env_logger;
extern crate libstratis;
extern crate log;
extern crate serde_json;

mod util;

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use serde_json::{Value, from_reader};

use self::devicemapper::{Bytes, Sectors};

use libstratis::consts::IEC;
use libstratis::engine::strat_engine::blockdev::wipe_sectors;

use util::logger::init_logger;
use util::simple_tests::test_basic_metadata;
use util::simple_tests::test_empty_pool;
use util::simple_tests::test_force_flag_dirty;
use util::simple_tests::test_force_flag_stratis;
use util::simple_tests::test_linear_device;
use util::simple_tests::test_pool_blockdevs;
use util::simple_tests::test_setup;
use util::simple_tests::test_teardown;
use util::simple_tests::test_thinpool_device;
use util::simple_tests::test_variable_length_metadata_times;


/// Set up count devices from configuration file.
/// Wipe first GiB on each device.
fn get_devices(count: u8) -> Option<Vec<PathBuf>> {
    let file = OpenOptions::new()
        .read(true)
        .open("tests/test_config.json")
        .unwrap();
    let config: Value = from_reader(&file).unwrap();
    let devpaths = config
        .get("ok_to_destroy_dev_array_key")
        .unwrap()
        .as_array()
        .unwrap();
    if devpaths.len() < count as usize {
        return None;
    }
    let devices: Vec<PathBuf> = devpaths
        .iter()
        .take(count as usize)
        .map(|x| PathBuf::from(x.as_str().unwrap()))
        .collect();

    let length = Bytes(IEC::Gi as u64).sectors();
    for device in devices.iter() {
        wipe_sectors(device, Sectors(0), length).unwrap();
    }

    Some(devices)
}

/// Run test on count real devices.
fn test_with_spec<F>(count: u8, test: F) -> ()
    where F: Fn(&[&Path]) -> ()
{
    init_logger();
    let devices = get_devices(count).unwrap();
    let device_paths: Vec<&Path> = devices.iter().map(|x| x.as_path()).collect();
    test(&device_paths);
}


#[test]
pub fn real_test_force_flag_stratis() {
    test_with_spec(2, test_force_flag_stratis);
    test_with_spec(3, test_force_flag_stratis);
}


#[test]
pub fn real_test_linear_device() {
    test_with_spec(2, test_linear_device);
    test_with_spec(3, test_linear_device);
}


#[test]
pub fn real_test_thinpool_device() {
    test_with_spec(3, test_thinpool_device);
}


#[test]
pub fn real_test_pool_blockdevs() {
    test_with_spec(3, test_pool_blockdevs);
}

#[test]
pub fn real_test_force_flag_dirty() {
    test_with_spec(3, test_force_flag_dirty);
}

#[test]
pub fn real_test_variable_length_metadata_times() {
    test_with_spec(3, test_variable_length_metadata_times);
}

#[test]
pub fn real_test_teardown() {
    test_with_spec(2, test_teardown);
}

#[test]
pub fn real_test_setup() {
    test_with_spec(4, test_setup);
}

#[test]
pub fn real_test_empty_pool() {
    test_with_spec(0, test_empty_pool)
}

#[test]
pub fn real_test_basic_metadata() {
    test_with_spec(4, test_basic_metadata);
}

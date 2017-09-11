// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate devicemapper;
extern crate env_logger;
extern crate libstratis;
extern crate log;
extern crate serde_json;

mod util;

use std::cmp;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use serde_json::{Value, from_reader};

use self::devicemapper::{Bytes, Sectors};

use libstratis::engine::IEC;
use libstratis::engine::strat_engine::device::wipe_sectors;

use util::logger::init_logger;
use util::blockdev_tests::test_blockdevmgr_used;
use util::blockdev_tests::test_force_flag_dirty;
use util::blockdev_tests::test_force_flag_stratis;
use util::blockdev_tests::test_pool_blockdevs;
use util::dm_tests::test_thinpool_device;
use util::dm_tests::test_linear_device;
use util::filesystem_tests::test_xfs_expand;
use util::pool_tests::test_filesystem_rename;
use util::pool_tests::test_thinpool_expand;
use util::pool_tests::test_thinpool_thindev_destroy;
use util::setup_tests::test_basic_metadata;
use util::setup_tests::test_initialize;
use util::setup_tests::test_pool_rename;
use util::setup_tests::test_pool_setup;
use util::setup_tests::test_setup;
use util::simple_tests::test_empty_pool;
use util::simple_tests::test_teardown;

pub struct RealTestDev {
    path: PathBuf,
}

impl RealTestDev {
    /// Construct a new test device for the given path.
    /// Wipe initial MiB to clear metadata.
    pub fn new(path: &Path) -> RealTestDev {
        wipe_sectors(path, Sectors(0), Bytes(IEC::Mi).sectors()).unwrap();
        RealTestDev { path: PathBuf::from(path) }
    }

    fn as_path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RealTestDev {
    fn drop(&mut self) {
        wipe_sectors(&self.path, Sectors(0), Bytes(IEC::Mi).sectors()).unwrap();
    }
}

enum DeviceLimits {
    Exactly(usize),
    AtLeast(usize),
    #[allow(dead_code)]
    Range(usize, usize), // inclusive
}

/// Get a list of counts of devices to use for tests.
/// None of the counts can be greater than avail.
fn get_device_counts(limits: DeviceLimits, avail: usize) -> Vec<usize> {
    // Convert enum to [lower, Option<upper>) values
    let (lower, maybe_upper) = match limits {
        DeviceLimits::Exactly(num) => (num, Some(num + 1)),
        DeviceLimits::AtLeast(num) => (num, None),
        DeviceLimits::Range(lower, upper) => {
            assert!(lower < upper);
            (lower, Some(upper + 1))
        }
    };

    let mut counts = vec![];

    // Check these values against available blockdevs
    if lower > avail {
        return counts;
    }

    counts.push(lower);

    if lower != avail {
        match maybe_upper {
            None => counts.push(avail),
            Some(upper) => {
                if lower + 1 < upper {
                    counts.push(cmp::min(upper - 1, avail))
                }
            }
        }
    }

    counts
}

/// Run test on real devices, using given constraints. Constraints may result
/// in multiple invocations of the test, with differing numbers of block
/// devices.
fn test_with_spec<F>(limits: DeviceLimits, test: F) -> ()
    where F: Fn(&[&Path]) -> ()
{
    let file = OpenOptions::new()
        .read(true)
        .open("tests/test_config.json")
        .unwrap();
    let config: Value = from_reader(&file).unwrap();
    let devpaths: Vec<_> = config
        .get("ok_to_destroy_dev_array_key")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|x| Path::new(x.as_str().unwrap()))
        .collect();

    let counts = get_device_counts(limits, devpaths.len());

    assert!(!counts.is_empty());

    init_logger();

    let runs = counts
        .iter()
        .map(|num| devpaths.iter().take(*num).collect::<Vec<_>>());

    for run_paths in runs {
        let devices: Vec<_> = run_paths.iter().map(|x| RealTestDev::new(x)).collect();
        test(&devices.iter().map(|x| x.as_path()).collect::<Vec<_>>());
    }
}


#[test]
pub fn real_test_force_flag_stratis() {
    test_with_spec(DeviceLimits::AtLeast(1), test_force_flag_stratis);
}


#[test]
pub fn real_test_linear_device() {
    test_with_spec(DeviceLimits::AtLeast(1), test_linear_device);
}


#[test]
pub fn real_test_thinpool_device() {
    test_with_spec(DeviceLimits::AtLeast(1), test_thinpool_device);
}

#[test]
pub fn real_test_thinpool_expand() {
    test_with_spec(DeviceLimits::AtLeast(1), test_thinpool_expand);
}

#[test]
pub fn real_test_thinpool_thindev_destroy() {
    test_with_spec(DeviceLimits::AtLeast(1), test_thinpool_thindev_destroy);
}

#[test]
pub fn real_test_pool_blockdevs() {
    test_with_spec(DeviceLimits::AtLeast(1), test_pool_blockdevs);
}

#[test]
pub fn real_test_force_flag_dirty() {
    test_with_spec(DeviceLimits::AtLeast(1), test_force_flag_dirty);
}

#[test]
pub fn real_test_teardown() {
    test_with_spec(DeviceLimits::AtLeast(1), test_teardown);
}

#[test]
pub fn real_test_initialize() {
    test_with_spec(DeviceLimits::AtLeast(2), test_initialize);
}

#[test]
pub fn real_test_empty_pool() {
    test_with_spec(DeviceLimits::Exactly(0), test_empty_pool)
}

#[test]
pub fn real_test_basic_metadata() {
    test_with_spec(DeviceLimits::AtLeast(2), test_basic_metadata);
}

#[test]
pub fn real_test_setup() {
    test_with_spec(DeviceLimits::AtLeast(2), test_setup);
}

#[test]
pub fn real_test_pool_rename() {
    test_with_spec(DeviceLimits::AtLeast(1), test_pool_rename);
}

#[test]
pub fn real_test_blockdevmgr_used() {
    test_with_spec(DeviceLimits::AtLeast(1), test_blockdevmgr_used);
}

#[test]
pub fn real_test_filesystem_rename() {
    test_with_spec(DeviceLimits::AtLeast(1), test_filesystem_rename);
}

#[test]
pub fn real_test_pool_setup() {
    test_with_spec(DeviceLimits::AtLeast(1), test_pool_setup);
}

#[test]
pub fn real_test_xfs_expand() {
    test_with_spec(DeviceLimits::AtLeast(1), test_xfs_expand);
}

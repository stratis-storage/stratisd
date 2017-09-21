// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate devicemapper;
extern crate env_logger;
extern crate libstratis;
extern crate log;
extern crate loopdev;
extern crate tempdir;

mod util;

use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use loopdev::{LoopControl, LoopDevice};
use tempdir::TempDir;

use devicemapper::{Bytes, Sectors};

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

/// Ways of specifying range of numbers of devices to use for tests.
/// Unlike real tests, there is no AtLeast constructor, as, at least in theory
/// there is no upper bound to the number of loop devices that can be made.
enum DeviceLimits {
    Exactly(usize),
    Range(usize, usize), // inclusive
}

pub struct LoopTestDev {
    ld: LoopDevice,
}

impl LoopTestDev {
    pub fn new(lc: &LoopControl, path: &Path) -> LoopTestDev {
        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .unwrap();

        // the proper way to do this is fallocate, but nix doesn't implement yet.
        // TODO: see https://github.com/nix-rust/nix/issues/596
        f.seek(SeekFrom::Start(IEC::Gi)).unwrap();
        f.write(&[0]).unwrap();
        f.flush().unwrap();

        let ld = lc.next_free().unwrap();
        ld.attach(path, 0).unwrap();
        // Wipe 1 MiB at the beginning, as data sits around on the files.
        wipe_sectors(&ld.get_path().unwrap(),
                     Sectors(0),
                     Bytes(IEC::Mi).sectors())
                .unwrap();

        LoopTestDev { ld: ld }
    }

    fn get_path(&self) -> PathBuf {
        self.ld.get_path().unwrap()
    }

    pub fn detach(&self) {
        self.ld.detach().unwrap()
    }
}

impl Drop for LoopTestDev {
    fn drop(&mut self) {
        self.detach()
    }
}

/// Get a list of counts of devices to use for tests.
fn get_device_counts(limits: DeviceLimits) -> Vec<usize> {
    match limits {
        DeviceLimits::Exactly(num) => vec![num],
        DeviceLimits::Range(lower, upper) => {
            assert!(lower < upper);
            vec![lower, upper]
        }
    }
}

/// Setup count loop backed devices in dir.
/// Make sure each loop device is backed by a 1 GiB file.
/// Wipe the first 1 MiB of the file.
fn get_devices(count: usize, dir: &TempDir) -> Vec<LoopTestDev> {
    let lc = LoopControl::open().unwrap();
    let mut loop_devices = Vec::new();

    for index in 0..count {
        let path = dir.path().join(format!("store{}", &index));
        loop_devices.push(LoopTestDev::new(&lc, &path));
    }
    loop_devices
}


/// Run the designated tests according to the specification.
fn test_with_spec<F>(limits: DeviceLimits, test: F) -> ()
    where F: Fn(&[&Path]) -> ()
{
    let counts = get_device_counts(limits);

    init_logger();

    for count in counts {
        let tmpdir = TempDir::new("stratis").unwrap();
        let loop_devices: Vec<LoopTestDev> = get_devices(count, &tmpdir);
        let device_paths: Vec<PathBuf> = loop_devices.iter().map(|x| x.get_path()).collect();
        let device_paths: Vec<&Path> = device_paths.iter().map(|x| x.as_path()).collect();
        test(&device_paths);
    }
}


#[test]
pub fn loop_test_force_flag_stratis() {
    test_with_spec(DeviceLimits::Range(1, 3), test_force_flag_stratis);
}


#[test]
pub fn loop_test_linear_device() {
    test_with_spec(DeviceLimits::Range(1, 3), test_linear_device);
}


#[test]
pub fn loop_test_thinpool_device() {
    /// This test requires more than 1 GiB.
    test_with_spec(DeviceLimits::Range(2, 3), test_thinpool_device);
}

#[test]
pub fn loop_test_thinpool_expand() {
    /// This test requires more than 1 GiB.
    test_with_spec(DeviceLimits::Range(2, 3), test_thinpool_expand);
}

#[test]
pub fn loop_test_thinpool_thindev_destroy() {
    /// This test requires more than 1 GiB.
    test_with_spec(DeviceLimits::Range(2, 3), test_thinpool_thindev_destroy);
}

#[test]
pub fn loop_test_pool_blockdevs() {
    /// This test requires more than 1 GiB.
    test_with_spec(DeviceLimits::Range(2, 3), test_pool_blockdevs);
}

#[test]
pub fn loop_test_force_flag_dirty() {
    test_with_spec(DeviceLimits::Range(1, 3), test_force_flag_dirty);
}

#[test]
pub fn loop_test_teardown() {
    test_with_spec(DeviceLimits::Range(1, 3), test_teardown);
}

#[test]
pub fn loop_test_initialize() {
    test_with_spec(DeviceLimits::Range(2, 3), test_initialize);
}

#[test]
pub fn loop_test_empty_pool() {
    test_with_spec(DeviceLimits::Exactly(0), test_empty_pool);
}

#[test]
pub fn loop_test_basic_metadata() {
    test_with_spec(DeviceLimits::Range(2, 3), test_basic_metadata);
}

#[test]
pub fn loop_test_setup() {
    test_with_spec(DeviceLimits::Range(2, 3), test_setup);
}

#[test]
pub fn loop_test_pool_rename() {
    test_with_spec(DeviceLimits::Range(1, 3), test_pool_rename);
}

#[test]
pub fn loop_test_blockdevmgr_used() {
    test_with_spec(DeviceLimits::Range(1, 3), test_blockdevmgr_used);
}

#[test]
pub fn loop_test_filesystem_rename() {
    test_with_spec(DeviceLimits::Range(1, 3), test_filesystem_rename);
}

#[test]
pub fn loop_test_pool_setup() {
    test_with_spec(DeviceLimits::Range(1, 3), test_pool_setup);
}

#[test]
pub fn loop_test_xfs_expand() {
    test_with_spec(DeviceLimits::Range(1, 3), test_xfs_expand);
}

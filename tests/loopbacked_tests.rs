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
use util::blockdev_tests::test_force_flag_dirty;
use util::blockdev_tests::test_force_flag_stratis;
use util::blockdev_tests::test_pool_blockdevs;
use util::dm_tests::test_thinpool_device;
use util::dm_tests::test_linear_device;
use util::setup_tests::test_basic_metadata;
use util::setup_tests::test_initialize;
use util::setup_tests::test_setup;
use util::simple_tests::test_empty_pool;
use util::simple_tests::test_teardown;

pub struct LoopTestDev {
    ld: LoopDevice,
}

impl LoopTestDev {
    pub fn new(lc: &LoopControl, path: &Path) -> LoopTestDev {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();

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

/// Setup count loop backed devices in dir.
/// Make sure each loop device is backed by a 1 GiB file.
/// Wipe the first 1 MiB of the file.
fn get_devices(count: u8, dir: &TempDir) -> Vec<LoopTestDev> {
    let lc = LoopControl::open().unwrap();
    let mut loop_devices = Vec::new();

    for index in 0..count {
        let path = dir.path().join(format!("store{}", &index));
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

        let ltd = LoopTestDev::new(&lc, &path);

        loop_devices.push(ltd);
    }
    loop_devices
}


/// Set up count loopbacked devices.
/// Then, run the designated test.
/// Then, take down the loop devices.
fn test_with_spec<F>(count: u8, test: F) -> ()
    where F: Fn(&[&Path]) -> ()
{
    init_logger();
    let tmpdir = TempDir::new("stratis").unwrap();
    let loop_devices: Vec<LoopTestDev> = get_devices(count, &tmpdir);
    let device_paths: Vec<PathBuf> = loop_devices.iter().map(|x| x.get_path()).collect();
    let device_paths: Vec<&Path> = device_paths.iter().map(|x| x.as_path()).collect();

    test(&device_paths);

}


#[test]
pub fn loop_test_force_flag_stratis() {
    test_with_spec(1, test_force_flag_stratis);
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
pub fn loop_test_teardown() {
    test_with_spec(2, test_teardown);
}

#[test]
pub fn loop_test_initialize() {
    test_with_spec(4, test_initialize);
}

#[test]
pub fn loop_test_empty_pool() {
    test_with_spec(0, test_empty_pool);
}

#[test]
pub fn loop_test_basic_metadata() {
    test_with_spec(4, test_basic_metadata);
}

#[test]
pub fn loop_test_setup() {
    test_with_spec(4, test_setup);
}

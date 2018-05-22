// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate loopdev;

use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::panic;
use std::path::{Path, PathBuf};

use tempfile;

use devicemapper::{Bytes, Sectors, IEC};

use self::loopdev::{LoopControl, LoopDevice};

use super::logger::init_logger;
use super::util::clean_up;

use super::super::device::wipe_sectors;

/// Ways of specifying range of numbers of devices to use for tests.
/// Unlike real tests, there is no AtLeast constructor, as, at least in theory
/// there is no upper bound to the number of loop devices that can be made.
/// The default value for size, if not specified, is 1 GiB.
pub enum DeviceLimits {
    /// Require exactly the number of devices specified.
    /// Specify their size in Sectors.
    Exactly(usize, Option<Sectors>),
    /// Required exactly the number of devices specified in the first and
    /// second argument to the constructors. Specify their size in Sectors.
    Range(usize, usize, Option<Sectors>),
}

pub struct LoopTestDev {
    ld: LoopDevice,
}

impl LoopTestDev {
    /// Create a new loopbacked device.
    /// Create its backing store of 1 GiB wiping the first 1 MiB.
    pub fn new(lc: &LoopControl, path: &Path, size: Option<Sectors>) -> LoopTestDev {
        let size = size.unwrap_or(Bytes(IEC::Gi).sectors());

        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .unwrap();

        // the proper way to do this is fallocate, but nix doesn't implement yet.
        // TODO: see https://github.com/nix-rust/nix/issues/596
        f.seek(SeekFrom::Start(*size.bytes())).unwrap();
        f.write(&[0]).unwrap();
        f.flush().unwrap();

        let ld = lc.next_free().unwrap();
        ld.attach_file(path).unwrap();
        // Wipe 1 MiB at the beginning, as data sits around on the files.
        wipe_sectors(&ld.path().unwrap(), Sectors(0), Bytes(IEC::Mi).sectors()).unwrap();

        LoopTestDev { ld }
    }
}

impl Drop for LoopTestDev {
    fn drop(&mut self) {
        self.ld.detach().unwrap()
    }
}

/// Get a list of counts of devices to use for tests.
fn get_device_counts(limits: DeviceLimits) -> Vec<(usize, Option<Sectors>)> {
    match limits {
        DeviceLimits::Exactly(num, size) => vec![(num, size)],
        DeviceLimits::Range(lower, upper, size) => {
            assert!(lower < upper);
            vec![(lower, size), (upper, size)]
        }
    }
}

/// Setup count loop backed devices in dir.
fn get_devices(count: usize, size: Option<Sectors>, dir: &tempfile::TempDir) -> Vec<LoopTestDev> {
    let lc = LoopControl::open().unwrap();
    let mut loop_devices = Vec::new();

    for index in 0..count {
        let path = dir.path().join(format!("store{}", &index));
        loop_devices.push(LoopTestDev::new(&lc, &path, size));
    }
    loop_devices
}

/// Run the designated tests according to the specification.
pub fn test_with_spec<F>(limits: DeviceLimits, test: F) -> ()
where
    F: Fn(&[&Path]) -> () + panic::RefUnwindSafe,
{
    let counts = get_device_counts(limits);

    init_logger();

    for (count, size) in counts {
        let tmpdir = tempfile::Builder::new()
            .prefix("stratis")
            .tempdir()
            .unwrap();
        let loop_devices: Vec<LoopTestDev> = get_devices(count, size, &tmpdir);
        let device_paths: Vec<PathBuf> =
            loop_devices.iter().map(|x| x.ld.path().unwrap()).collect();
        let device_paths: Vec<&Path> = device_paths.iter().map(|x| x.as_path()).collect();

        clean_up().unwrap();

        let result = panic::catch_unwind(|| {
            test(&device_paths);
        });
        let tear_down = clean_up();

        result.unwrap();
        tear_down.unwrap();
    }
}

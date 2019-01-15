// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate loopdev;

use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::panic;
use std::path::{Path, PathBuf};

use nix;
use tempfile;

use devicemapper::{Bytes, Sectors, IEC};

use self::loopdev::{LoopControl, LoopDevice};

use crate::engine::strat_engine::tests::logger::init_logger;
use crate::engine::strat_engine::tests::util::clean_up;

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
    /// Create its backing store of specified size. The file is sparse but
    /// will appear to be zeroed.
    pub fn new(lc: &LoopControl, path: &Path, size: Option<Sectors>) -> LoopTestDev {
        let size = size.unwrap_or_else(|| Bytes(IEC::Gi).sectors());

        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .unwrap();

        nix::unistd::ftruncate(f.as_raw_fd(), *size.bytes() as nix::libc::off_t).unwrap();
        f.sync_all().unwrap();

        let ld = lc.next_free().unwrap();
        ld.attach_file(path).unwrap();

        LoopTestDev { ld }
    }
}

impl Drop for LoopTestDev {
    fn drop(&mut self) {
        self.ld.detach().unwrap()
    }
}

/// Get a list of counts of devices to use for tests.
fn get_device_counts(limits: &DeviceLimits) -> Vec<(usize, Option<Sectors>)> {
    match limits {
        DeviceLimits::Exactly(num, size) => vec![(*num, *size)],
        DeviceLimits::Range(lower, upper, size) => {
            assert!(lower < upper);
            vec![(*lower, *size), (*upper, *size)]
        }
    }
}

/// Setup count loop backed devices in dir of specified size.
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
pub fn test_with_spec<F>(limits: &DeviceLimits, test: F)
where
    F: Fn(&[&Path]) -> () + panic::RefUnwindSafe,
{
    let counts = get_device_counts(&limits);

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

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    env,
    fs::{File, OpenOptions},
    mem::forget,
    os::unix::io::AsRawFd,
    panic,
    path::{Path, PathBuf},
};

use loopdev::{LoopControl, LoopDevice};

use devicemapper::{Bytes, Sectors, IEC};

use crate::{
    engine::strat_engine::tests::{logger::init_logger, util::clean_up},
    stratis::StratisResult,
};

/// Ways of specifying range of numbers of devices to use for tests.
/// Unlike real tests, there is no AtLeast constructor, as, at least in theory
/// there is no upper bound to the number of loop devices that can be made.
/// The default value for size, if not specified, is 1 GiB.
pub enum DeviceLimits {
    /// Require exactly the number of devices specified.
    /// Specify their size in Sectors.
    Exactly(usize, Option<Sectors>),
    /// Specify a minimum and maximum number of devices in the first and
    /// second argument to the constructors. Specify their size in Sectors.
    Range(usize, usize, Option<Sectors>),
}

pub struct LoopTestDev {
    ld: LoopDevice,
    backing_file: File,
}

impl LoopTestDev {
    /// Create a new loopbacked device.
    /// Create its backing store of specified size. The file is sparse but
    /// will appear to be zeroed.
    pub fn new(lc: &LoopControl, path: &Path, size: Option<Sectors>) -> LoopTestDev {
        let size = size.unwrap_or_else(|| Bytes::from(IEC::Gi).sectors());

        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .unwrap();

        nix::unistd::ftruncate(
            f.as_raw_fd(),
            convert_test!(*size.bytes(), u128, nix::libc::off_t),
        )
        .unwrap();
        f.sync_all().unwrap();

        let ld = lc.next_free().unwrap();
        ld.attach_file(path).unwrap();

        LoopTestDev {
            ld,
            backing_file: f,
        }
    }

    /// Grow the device size by a factor of 2.
    pub fn grow(&self) -> StratisResult<()> {
        let current_len = self.backing_file.metadata()?.len();
        self.backing_file.set_len(2 * current_len)?;
        self.ld.set_capacity()?;
        Ok(())
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
    F: Fn(&[&Path]) + panic::RefUnwindSafe,
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

        let tear_down = if env::var("NO_TEST_CLEAN_UP") != Ok("1".to_string()) {
            Some(clean_up())
        } else {
            forget(tmpdir);
            loop_devices.into_iter().for_each(forget);
            None
        };

        result.unwrap();

        if let Some(td) = tear_down {
            td.unwrap();
        }
    }
}

/// Run the designated tests according to the specification with one function being
/// executed before the first device is doubled and one after.
pub fn test_device_grow_with_spec<F1, F2>(limits: &DeviceLimits, test: F1, test_after_grow: F2)
where
    F1: Fn(&[&Path]) + panic::RefUnwindSafe,
    F2: Fn(&[&Path]) + panic::RefUnwindSafe,
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

        let result_pre_grow = panic::catch_unwind(|| {
            test(&device_paths);
        });

        let loop_dev_grow_result = loop_devices.first().map(|l| l.grow());

        let result_post_grow = if result_pre_grow.is_ok() {
            panic::catch_unwind(|| test_after_grow(&device_paths))
        } else {
            Ok(())
        };

        let tear_down = if env::var("NO_TEST_CLEAN_UP") != Ok("1".to_string()) {
            Some(clean_up())
        } else {
            forget(tmpdir);
            loop_devices.into_iter().for_each(forget);
            None
        };

        loop_dev_grow_result.unwrap().unwrap();

        result_pre_grow.unwrap();
        result_post_grow.unwrap();

        if let Some(td) = tear_down {
            td.unwrap();
        }
    }
}

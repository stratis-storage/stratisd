// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::{cmp, panic};

use serde_json::{from_reader, Value};

use devicemapper::{Bytes, Sectors, IEC};

use super::super::backstore::blkdev_size;
use super::super::device::wipe_sectors;

use super::logger::init_logger;
use super::util::clean_up;

pub struct RealTestDev {
    path: PathBuf,
}

impl RealTestDev {
    /// Construct a new test device for the given path.
    /// Wipe initial MiB to clear metadata.
    pub fn new(path: &Path) -> RealTestDev {
        wipe_sectors(path, Sectors(0), Bytes(IEC::Mi).sectors()).unwrap();
        RealTestDev {
            path: PathBuf::from(path),
        }
    }

    /// Get the device node of the device.
    fn as_path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RealTestDev {
    fn drop(&mut self) {
        wipe_sectors(&self.path, Sectors(0), Bytes(IEC::Mi).sectors()).unwrap();
    }
}

/// Shorthand for specifying devices to use for tests
pub enum DeviceLimits {
    /// Use exactly the number of devices specified
    Exactly(usize),
    /// Use at least the number of devices specified, but if there are more
    /// devices available, also use the maximum number of devices.
    AtLeast(usize),
    /// Use exactly the number of devices specified in the first argument,
    /// and the minimum of the number of devices specified and the number
    /// of devices available in the second argument.
    Range(usize, usize),
}

/// Get a list of lists of devices to use for tests.
/// May return an empty list if the request is not satisfiable.
fn get_device_runs<'a>(
    limits: DeviceLimits,
    dev_sizes: &[(&'a Path, Sectors)],
) -> Vec<Vec<&'a Path>> {
    let avail = dev_sizes.len();

    // Convert enum to [lower, Option<upper>) values
    let (lower, maybe_upper) = match limits {
        DeviceLimits::Exactly(num) => (num, Some(num + 1)),
        DeviceLimits::AtLeast(num) => (num, None),
        DeviceLimits::Range(lower, upper) => {
            assert!(lower < upper);
            (lower, Some(upper + 1))
        }
    };

    let mut device_lists = vec![];

    // Check these values against available blockdevs
    if lower > avail {
        return device_lists;
    }

    device_lists.push(
        dev_sizes
            .iter()
            .take(lower)
            .map(|&(d, _)| d)
            .collect::<Vec<_>>(),
    );

    if lower != avail {
        match maybe_upper {
            None => device_lists.push(
                dev_sizes
                    .iter()
                    .take(avail)
                    .map(|&(d, _)| d)
                    .collect::<Vec<_>>(),
            ),
            Some(upper) => {
                if lower + 1 < upper {
                    device_lists.push(
                        dev_sizes
                            .iter()
                            .take(cmp::min(upper - 1, avail))
                            .map(|&(d, _)| d)
                            .collect::<Vec<_>>(),
                    )
                }
            }
        }
    }

    device_lists
}

/// Run test on real devices, using given constraints. Constraints may result
/// in multiple invocations of the test, with differing numbers of block
/// devices.
pub fn test_with_spec<F>(limits: DeviceLimits, test: F) -> ()
where
    F: Fn(&[&Path]) -> () + panic::RefUnwindSafe,
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

    let dev_sizes: Vec<(&Path, Sectors)> = devpaths
        .iter()
        .map(|p| {
            (
                *p,
                blkdev_size(&OpenOptions::new().read(true).open(p).unwrap())
                    .unwrap()
                    .sectors(),
            )
        })
        .collect();

    let runs = get_device_runs(limits, &dev_sizes);

    assert!(!runs.is_empty());

    init_logger();

    for run_paths in runs {
        let devices: Vec<_> = run_paths.iter().map(|x| RealTestDev::new(x)).collect();

        clean_up().unwrap();

        let result =
            panic::catch_unwind(|| test(&devices.iter().map(|x| x.as_path()).collect::<Vec<_>>()));
        let tear_down = clean_up();

        result.unwrap();
        tear_down.unwrap();
    }
}

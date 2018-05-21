// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate either;

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::{cmp, panic};

use self::either::Either;
use serde_json::{from_reader, Value};

use devicemapper::{Bytes, DmDevice, LinearDev, Sectors, IEC};

use super::super::backstore::blkdev_size;
use super::super::device::wipe_sectors;

use super::logger::init_logger;
use super::util::clean_up;

pub struct RealTestDev {
    dev: Either<PathBuf, LinearDev>,
}

impl RealTestDev {
    /// Construct a new test device for the given path.
    /// Wipe initial MiB to clear metadata.
    pub fn new(path: &Path) -> RealTestDev {
        wipe_sectors(path, Sectors(0), Bytes(IEC::Mi).sectors()).unwrap();
        RealTestDev {
            dev: Either::Left(PathBuf::from(path)),
        }
    }

    /// Get the device node of the device.
    fn as_path(&self) -> PathBuf {
        self.dev.as_ref().either(|p| p.clone(), |l| l.devnode())
    }
}

impl Drop for RealTestDev {
    fn drop(&mut self) {
        wipe_sectors(&self.as_path(), Sectors(0), Bytes(IEC::Mi).sectors()).unwrap();
    }
}

/// Shorthand for specifying devices to use for tests
/// For the minimum size, if None is given, the default 1 GiB is chosen.
pub enum DeviceLimits {
    /// Use exactly the number of devices specified
    /// The second argument is the minimum size for all devices.
    /// The third argument is the maximum size for all devices.
    Exactly(usize, Option<Sectors>, Option<Sectors>),
    /// Use at least the number of devices specified, but if there are more
    /// devices available, also use the maximum number of devices.
    /// The second argument is the minimum size for all devices.
    /// The third argument is the maximum size for all devices.
    AtLeast(usize, Option<Sectors>, Option<Sectors>),
    /// Use exactly the number of devices specified in the first argument,
    /// and the minimum of the number of devices specified and the number
    /// of devices available in the second argument.
    /// The third argument is the minimum size for all devices.
    /// The fourth argument is the maximum size for all devices.
    Range(usize, usize, Option<Sectors>, Option<Sectors>),
}

/// Get a list of lists of devices to use for tests.
/// May return an empty list if the request is not satisfiable.
fn get_device_runs<'a>(
    limits: DeviceLimits,
    dev_sizes: &[(&'a Path, Sectors)],
) -> Vec<Vec<&'a Path>> {
    let (lower, maybe_upper, min_size, max_size) = match limits {
        DeviceLimits::Exactly(num, min_size, max_size) => (num, Some(num + 1), min_size, max_size),
        DeviceLimits::AtLeast(num, min_size, max_size) => (num, None, min_size, max_size),
        DeviceLimits::Range(lower, upper, min_size, max_size) => {
            assert!(lower < upper);
            (lower, Some(upper + 1), min_size, max_size)
        }
    };

    let min_size = min_size.unwrap_or(Bytes(IEC::Gi).sectors());

    let (matches, _): (Vec<(&Path, Sectors)>, Vec<(&Path, Sectors)>) =
        dev_sizes.iter().partition(|&(_, s)| *s >= min_size);

    // If there is not a sufficient number of devices large enough to match
    // the lower bound, return an empty vec.
    if lower > matches.len() {
        return vec![];
    }

    let (matches, _): (Vec<(&Path, Sectors)>, Vec<(&Path, Sectors)>) = if max_size.is_none() {
        (matches, vec![])
    } else {
        let max_size = max_size.expect("!max_size.is_none()");
        matches.iter().partition(|&(_, s)| *s <= max_size)
    };

    let avail = matches.len();

    // If there is not a sufficient number of devices small enough to match
    // the lower bound, return an empty vec.
    if lower > avail {
        return vec![]; // FIXME: generate more devices
    }

    let mut device_lists = vec![];

    device_lists.push(
        matches
            .iter()
            .take(lower)
            .map(|&(d, _)| d)
            .collect::<Vec<_>>(),
    );

    if lower != avail {
        match maybe_upper {
            None => device_lists.push(
                matches
                    .iter()
                    .take(avail)
                    .map(|&(d, _)| d)
                    .collect::<Vec<_>>(),
            ),
            Some(upper) => {
                if lower + 1 < upper {
                    device_lists.push(
                        matches
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

        let paths: Vec<PathBuf> = devices.iter().map(|x| x.as_path()).collect();
        let paths: Vec<&Path> = paths.iter().map(|x| x.as_path()).collect();
        let result = panic::catch_unwind(|| test(&paths));
        let tear_down = clean_up();

        result.unwrap();
        tear_down.unwrap();
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate serde_json;

use std::cmp;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use serde_json::{Value, from_reader};

use devicemapper::{Bytes, IEC, Sectors};

use super::logger::init_logger;
use super::util::clean_up;

use super::super::device::wipe_sectors;

pub struct RealTestDev {
    path: PathBuf,
}

impl RealTestDev {
    /// Construct a new test device for the given path.
    /// Wipe initial MiB to clear metadata.
    pub fn new(path: &Path) -> RealTestDev {
        clean_up();
        wipe_sectors(path, Sectors(0), Bytes(IEC::Mi).sectors()).unwrap();
        RealTestDev { path: PathBuf::from(path) }
    }

    /// Get the device node of the device.
    fn as_path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RealTestDev {
    fn drop(&mut self) {
        clean_up();
        wipe_sectors(&self.path, Sectors(0), Bytes(IEC::Mi).sectors()).unwrap();
    }
}

pub enum DeviceLimits {
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
pub fn test_with_spec<F>(limits: DeviceLimits, test: F) -> ()
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

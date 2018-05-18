// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate either;

use std::panic;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use serde_json::{Value, from_reader};

use devicemapper::{Bytes, DevId, Device, devnode_to_devno, DmDevice, DmFlags, DmName, LinearDev,
                   LinearDevTargetParams, LinearTargetParams, Sectors, TargetLine, IEC};

use super::logger::init_logger;
use super::util::clean_up;

use super::super::backstore::blkdev_size;
use super::super::device::wipe_sectors;
use super::super::dm::get_dm;

use self::either::Either;

pub struct RealTestDev {
    dev: Either<PathBuf, LinearDev>,
}

impl RealTestDev {
    /// Construct a new test device for the given path.
    /// Wipe initial MiB to clear metadata.
    pub fn new(dev: Either<PathBuf, LinearDev>) -> RealTestDev {
        let result = RealTestDev { dev };
        wipe_sectors(result.as_path(), Sectors(0), Bytes(IEC::Mi).sectors()).unwrap();
        result
    }

    /// Get the device node of the device.
    fn as_path(&self) -> PathBuf {
        self.dev.as_ref().either(|p| p.clone(), |l| l.devnode())
    }
}

impl Drop for RealTestDev {
    fn drop(&mut self) {
        wipe_sectors(&self.as_path(), Sectors(0), Bytes(IEC::Mi).sectors()).unwrap();
        // If the block device is a LinearDev clean up
        if let Some(ref ld) = self.dev.as_ref().right() {
            // LinearDev::teardown() can't be called from a class that implements
            // drop.  TODO: Is there a better work around?
            get_dm()
                .device_remove(&DevId::Name(ld.name()), DmFlags::empty())
                .unwrap();
        }
    }
}

pub enum DeviceLimits {
    Exactly(usize, Option<Sectors>, Option<Sectors>),
    AtLeast(usize, Option<Sectors>, Option<Sectors>),
    Range(usize, usize), // inclusive
}


/// Create count LinearDevs of min_size from dev. Return a vector of the
/// newly created LinearDevs.
/// Precondition: min_size * count < the size of the device
fn slice_disk(dev: &Path, min_size: Sectors, count: u64) -> Vec<LinearDev> {
    let mut start = Sectors(0);
    let mut lds = vec![];

    for i in 0..count {
        let params = LinearTargetParams::new(Device::from(devnode_to_devno(dev).unwrap().unwrap()),
                                             start);
        let table =
            vec![TargetLine::new(Sectors(0), min_size, LinearDevTargetParams::Linear(params))];
        let ld =
            LinearDev::setup(get_dm(),
                             DmName::new(&format!("stratis_test_{}", i)).expect("valid format"),
                             None,
                             table.clone())
                    .unwrap();
        start = start + min_size;
        lds.push(ld)
    }

    lds
}

/// Create exactly min_count LinearDevs of min_size from the paths in
/// dev_sizes. If this is impossible, return an empty Vec.
fn slice_devices(dev_sizes: &[(&Path, Sectors)],
                 min_count: usize,
                 min_size: Sectors)
                 -> Vec<RealTestDev> {
    // The number of devices of min_size that each device can provide
    let path_slice_count = dev_sizes
        .iter()
        .map(|(dev, size)| (dev, *size / min_size))
        .collect::<Vec<(_, u64)>>();

    // Get the sum of all the block devices for each path
    let total_possible_slices = path_slice_count
        .iter()
        .fold(0u64, |sum, tup| sum + tup.1);

    // If the min_count can be provided, create linear devs to be returned,
    // Otherwise return an empty vec[].
    if total_possible_slices >= min_count as u64 {

        // Determine how many of the paths we need to slice into LinearDevs to
        // meet min_count
        let mut needed_count = 0;
        let mut avail_slices: u64 = 0;
        for path in path_slice_count.iter() {
            needed_count += 1;
            avail_slices += path.1;
            if avail_slices >= min_count as u64 {
                break;
            }
        }

        let mut linear_devs: Vec<LinearDev> = path_slice_count[0..needed_count]
            .iter()
            .map(|&tup| slice_disk(tup.0, min_size, tup.1))
            .flat_map(|vec| vec.into_iter())
            .collect();

        // Teardown any extra linear devs
        for ld in linear_devs.split_off(min_count) {
            ld.teardown(get_dm()).unwrap();
        }

        // Return the requested amount as RealTestDevs
        linear_devs
            .into_iter()
            .map(|ld| RealTestDev::new(Either::Right(ld)))
            .collect()
    } else {
        vec![]
    }
}

/// Get a list of test devices to be used for tests.
fn get_devices(limits: DeviceLimits, devpaths: &[&Path]) -> Vec<Vec<RealTestDev>> {
    // Convert enum to [lower, Option<upper>, min_size, max_size) values
    let (lower, maybe_upper, min_size, max_size) = match limits {
        DeviceLimits::Exactly(num, min_size, max_size) => (num, None, min_size, max_size),
        DeviceLimits::AtLeast(num, min_size, max_size) => (num, None, min_size, max_size),
        DeviceLimits::Range(lower, upper) => {
            assert!(lower < upper,
                    "Upper bound of range must be greater than the lower bound");
            (lower, Some(upper + 1), None, None)
        }
    };
    let min_size = min_size.unwrap_or(Bytes(IEC::Gi).sectors());

    assert!(max_size.is_none() || Some(min_size) <= max_size,
            "Minimum device size greater than maximum");

    let dev_sizes: Vec<(&Path, Sectors)> = devpaths
        .iter()
        .map(|p| {
                 (*p,
                  blkdev_size(&OpenOptions::new().read(true).open(p).unwrap())
                      .unwrap()
                      .sectors())
             })
        .collect();

    let test_devices = dev_sizes
        .iter()
        .filter(|(_, s)| min_size < *s && (max_size.is_none() || Some(*s) <= max_size))
        .map(|(p, _)| *p)
        .collect::<Vec<&Path>>();

    let mut devices: Vec<Vec<RealTestDev>> = vec![];

    // FIXME: if it is necessary to partition the devices using linear devs,
    // only return one list of devices, ever. mulhern believes that the only
    // way to be able to make a selection of devices for an upper and lower
    // bound is to make LinearDevs Clone. If LinearDevs were Clone, then it
    // would be possible to clone portions of a vec of RealTestDevs for the
    // upper and lower bounds. If this is fixed, then Range constructor
    // can be extended to include size bounds.
    if test_devices.len() < lower {
        let test_devices = slice_devices(&dev_sizes, lower, min_size);
        assert!(test_devices.len() == lower,
                "Test devices supplied do not meet minimum requirements");
        devices.push(test_devices);
    } else {
        devices.push(test_devices[0..lower]
                         .to_vec()
                         .iter()
                         .map(|x| RealTestDev::new(Either::Left(x.to_path_buf())))
                         .collect());
        if maybe_upper.is_some() {
            devices.push(test_devices[0..(maybe_upper.unwrap() - 1)]
                             .to_vec()
                             .iter()
                             .map(|x| RealTestDev::new(Either::Left(x.to_path_buf())))
                             .collect());
        };
    }
    devices
}

/// Run test on real devices, using given constraints. Constraints may result
/// in multiple invocations of the test, with differing numbers of block
/// devices.
pub fn test_with_spec<F>(limits: DeviceLimits, test: F) -> ()
    where F: Fn(&[&Path]) -> () + panic::RefUnwindSafe
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

    let runs = get_devices(limits, &devpaths);

    assert!(!runs.is_empty());

    init_logger();

    for devices in runs {
        clean_up().unwrap();

        let paths: Vec<PathBuf> = devices.iter().map(|x| x.as_path()).collect();
        let paths: Vec<&Path> = paths.iter().map(|x| x.as_path()).collect();
        let result = panic::catch_unwind(|| test(&paths));
        let tear_down = clean_up();

        result.unwrap();
        tear_down.unwrap();
    }
}

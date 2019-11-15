// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    cmp,
    fs::OpenOptions,
    panic,
    path::{Path, PathBuf},
};

use either::Either;
use serde_json::{from_reader, Value};
use uuid::Uuid;

use devicemapper::{
    devnode_to_devno, Bytes, Device, DmDevice, DmName, LinearDev, LinearDevTargetParams,
    LinearTargetParams, Sectors, TargetLine, IEC,
};

use crate::engine::strat_engine::{
    device::{blkdev_size, wipe_sectors},
    dm::get_dm,
    tests::{logger::init_logger, util::clean_up},
};

pub struct RealTestDev {
    dev: Either<PathBuf, LinearDev>,
}

impl RealTestDev {
    /// Construct a new test device for the given path.
    /// Wipe initial MiB to clear metadata.
    pub fn new(dev: Either<PathBuf, LinearDev>) -> RealTestDev {
        let test_dev = RealTestDev { dev };
        wipe_sectors(test_dev.as_path(), Sectors(0), Bytes(IEC::Mi).sectors()).unwrap();
        test_dev
    }

    /// Get the device node of the device.
    fn as_path(&self) -> PathBuf {
        self.dev.as_ref().either(|p| p.clone(), |l| l.devnode())
    }

    /// Teardown a real test dev
    fn teardown(self) {
        wipe_sectors(&self.as_path(), Sectors(0), Bytes(IEC::Mi).sectors()).unwrap();
        if let Some(mut ld) = self.dev.right() {
            ld.teardown(get_dm()).unwrap();
        }
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
    #[allow(dead_code)]
    Range(usize, usize, Option<Sectors>, Option<Sectors>),
}

/// Get a list of lists of devices to use for tests.
/// May return an empty list if the request is not satisfiable.
#[allow(clippy::type_complexity)]
fn get_device_runs<'a>(
    limits: &DeviceLimits,
    dev_sizes: &[(&'a Path, Sectors)],
) -> Vec<Vec<(&'a Path, Option<(Sectors, Sectors)>)>> {
    let (lower, maybe_upper, min_size, max_size) = match limits {
        DeviceLimits::Exactly(num, min_size, max_size) => (*num, Some(*num), *min_size, *max_size),
        DeviceLimits::AtLeast(num, min_size, max_size) => (*num, None, *min_size, *max_size),
        DeviceLimits::Range(lower, upper, min_size, max_size) => {
            assert!(lower < upper);
            (*lower, Some(*upper), *min_size, *max_size)
        }
    };

    let min_size = min_size.unwrap_or_else(|| Bytes(IEC::Gi).sectors());

    assert!(max_size.is_none() || Some(min_size) <= max_size);

    // Retain only devices that are larger than min_size
    let (matches, _): (Vec<(&Path, Sectors)>, Vec<(&Path, Sectors)>) =
        dev_sizes.iter().partition(|&&(_, s)| s >= min_size);

    // If there is not a sufficient number of devices large enough to match
    // the lower bound, return an empty vec. TODO: It would be possible to try
    // harder, by merging several devices into a single linear device. If this
    // turns out to be necessary, this is the correct place to do it.
    if lower > matches.len() {
        return vec![];
    }

    // Retain only devices that are less than max_size
    let (mut matches, mut too_large): (Vec<(&Path, Sectors)>, Vec<(&Path, Sectors)>) =
        if max_size.is_none() {
            (matches, vec![])
        } else {
            let max_size = max_size.expect("!max_size.is_none()");
            matches.iter().partition(|&&(_, s)| s <= max_size)
        };

    let avail = matches.len();
    let needed = maybe_upper.unwrap_or(lower);
    let must_generate = if avail > needed { 0 } else { needed - avail };
    let avail_specs = {
        let mut avail_specs = vec![];
        while avail_specs.len() < must_generate && (!too_large.is_empty() || !matches.is_empty()) {
            let (path, size) = if too_large.is_empty() {
                matches.pop().expect("!matches.is_empty()")
            } else {
                too_large.pop().expect("!too_large.is_empty()")
            };
            let mut new_pairs = vec![];
            for i in 0..size / min_size {
                new_pairs.push((path, Some((i * min_size, min_size))))
            }
            avail_specs.extend(new_pairs);
        }
        avail_specs.extend(matches.iter().map(|&(p, _)| (p, None)));
        avail_specs
    };

    let avail = avail_specs.len();

    // If there is not a sufficient number of devices small enough to match
    // the lower bound, return an empty vec.
    if lower > avail {
        return vec![];
    }

    let mut device_lists = vec![];

    device_lists.push(avail_specs.iter().take(lower).cloned().collect::<Vec<_>>());

    if lower != avail {
        match maybe_upper {
            None => device_lists.push(avail_specs.iter().take(avail).cloned().collect::<Vec<_>>()),
            Some(upper) => {
                if lower < upper {
                    device_lists.push(
                        avail_specs
                            .iter()
                            .take(cmp::min(upper, avail))
                            .cloned()
                            .collect::<Vec<_>>(),
                    )
                }
            }
        }
    }

    device_lists
}

/// Make a new LinearDev according to specs
fn make_linear_test_dev(devnode: &Path, start: Sectors, length: Sectors) -> LinearDev {
    let params = LinearTargetParams::new(
        Device::from(devnode_to_devno(devnode).unwrap().unwrap()),
        start,
    );
    let table = vec![TargetLine::new(
        Sectors(0),
        length,
        LinearDevTargetParams::Linear(params),
    )];
    LinearDev::setup(
        get_dm(),
        DmName::new(&format!("stratis_test_{}", Uuid::new_v4())).expect("valid format"),
        None,
        table,
    )
    .unwrap()
}

/// Run test on real devices, using given constraints. Constraints may result
/// in multiple invocations of the test, with differing numbers of block
/// devices.
pub fn test_with_spec<F>(limits: &DeviceLimits, test: F)
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
        clean_up().unwrap();

        let mut devices: Vec<_> = run_paths
            .iter()
            .map(|&(path, spec)| match spec {
                Some((start, length)) => {
                    RealTestDev::new(Either::Right(make_linear_test_dev(path, start, length)))
                }
                None => RealTestDev::new(Either::Left(path.to_path_buf())),
            })
            .collect();

        let paths: Vec<PathBuf> = devices.iter().map(|x| x.as_path()).collect();
        let paths: Vec<&Path> = paths.iter().map(|x| x.as_path()).collect();
        let result = panic::catch_unwind(|| test(&paths));
        let tear_down = clean_up();

        result.unwrap();
        tear_down.unwrap();

        for dev in devices.drain(..) {
            dev.teardown();
        }
    }
}

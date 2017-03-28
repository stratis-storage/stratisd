// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate libudev;
extern crate loopdev;

use std::path::PathBuf;

use self::libudev::{Context, Enumerator};

#[allow(dead_code)]
/// Returns the device nodes of all loop devices.
/// Loop devices are block devices with major number 7.
pub fn loop_devices<'a>() -> Vec<PathBuf> {
    let context = Context::new().unwrap();
    let mut enumerator = Enumerator::new(&context).unwrap();
    enumerator.match_property("MAJOR", "7").unwrap();
    enumerator.match_subsystem("block").unwrap();
    enumerator.scan_devices().unwrap().map(|x| x.devnode().unwrap().to_path_buf()).collect()
}

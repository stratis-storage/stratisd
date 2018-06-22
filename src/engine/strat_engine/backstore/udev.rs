// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Udev dependent code for getting information about devices.

use std::ffi::OsStr;
use std::path::Path;

use libudev;

use stratis::{StratisError, StratisResult};

use super::super::super::udev::get_udev;

/// Get a udev property with the given name for the given device.
/// Returns an error if the value of the property can not be converted
/// to a String using the standard conversion for this OS.
pub fn get_udev_property<T: AsRef<OsStr>>(
    device: &libudev::Device,
    property_name: T,
) -> StratisResult<Option<String>> {
    match device.property_value(property_name) {
        Some(value) => match value.to_str() {
            Some(value) => Ok(Some(value.into())),
            None => Err(StratisError::Error(format!(
                "Unable to convert {:?} to str",
                value
            ))),
        },
        None => Ok(None),
    }
}

/// Lookup the WWN from the udev db using the device node eg. /dev/sda
pub fn hw_lookup(dev_node_search: &Path) -> StratisResult<Option<String>> {
    #![allow(let_and_return)]
    let context = get_udev();
    let mut enumerator = libudev::Enumerator::new(&context)?;
    enumerator.match_subsystem("block")?;
    enumerator.match_property("DEVTYPE", "disk")?;

    let result = enumerator
        .scan_devices()?
        .find(|x| x.devnode().map_or(false, |d| dev_node_search == d))
        .map_or(Ok(None), |dev| get_udev_property(&dev, "ID_WWN"));

    result
}

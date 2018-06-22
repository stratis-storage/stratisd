// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Udev dependent code for getting information about devices.

use std::ffi::OsStr;
use std::path::Path;

use libudev;

use stratis::{StratisError, StratisResult};

use super::super::super::udev::get_udev;

/// If the expression is true, then it seems that no other system is
/// known to udev to claim this device.
/// Note from mulhern: I have no idea myself why this particular expression
/// should be correct. The expression is equivalent to that used in PR:
/// https://github.com/stratis-storage/stratisd/pull/936.
#[allow(dead_code)]
pub fn unclaimed(device: &libudev::Device) -> StratisResult<bool> {
    Ok((get_udev_property(device, "ID_PART_TABLE_TYPE")?.is_none()
        || get_udev_property(device, "ID_PART_ENTRY_DISK")?.is_some())
        && get_udev_property(device, "ID_FS_USAGE")?.is_none())
}

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

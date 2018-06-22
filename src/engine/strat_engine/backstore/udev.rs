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

/// Locate a udev block device by using its device node and apply a function
/// to it, returning the result of the function.
/// This approach is necessitated by the libudev lifetimes, which do not
/// allow returning anything directly obtained from the enumerator value
/// created in the method itself.
pub fn udev_block_device_apply<F, U>(devnode: &Path, f: F) -> StratisResult<Option<U>>
where
    F: FnOnce(&libudev::Device<'_>) -> U,
{
    #![allow(let_and_return)]
    let context = get_udev();

    let mut enumerator = libudev::Enumerator::new(context)?;
    enumerator.match_subsystem("block")?;

    let result = match enumerator
        .scan_devices()?
        .find(|x| x.devnode().map_or(false, |d| devnode == d))
    {
        Some(device) => Ok(Some(f(&device))),
        None => Ok(None),
    };
    result
}

/// Lookup the WWN from the udev db using the device node.
/// Returns an error if there was an error looking up the device.
/// Returns None if the device could not be found.
/// Returns Ok(Some(Err(...))) if the device was found but there was an
/// error interpreting the value.
/// Returns Ok(Some(Ok(None))) if there was no ID_WNN in the database.
pub fn hw_lookup(dev_node_search: &Path) -> StratisResult<Option<StratisResult<Option<String>>>> {
    udev_block_device_apply(dev_node_search, |dev| get_udev_property(dev, "ID_WWN"))
}

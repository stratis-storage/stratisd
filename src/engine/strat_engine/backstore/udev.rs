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
pub fn unclaimed(device: &libudev::Device) -> bool {
    get_udev_property(device, "DM_MULTIPATH_DEVICE_PATH").is_none()
        && (get_udev_property(device, "ID_PART_TABLE_TYPE").is_none()
            || get_udev_property(device, "ID_PART_ENTRY_DISK").is_some())
        && get_udev_property(device, "ID_FS_USAGE").is_none()
}

/// Get a udev property with the given name for the given device.
/// Returns None if no udev property found for the given property name.
/// Returns an error if the value of the property can not be converted
/// to a String using the standard conversion for this OS.
pub fn get_udev_property<T: AsRef<OsStr>>(
    device: &libudev::Device,
    property_name: T,
) -> Option<StratisResult<String>> {
    match device.property_value(property_name) {
        Some(value) => match value.to_str() {
            Some(value) => Some(Ok(value.into())),
            None => Some(Err(StratisError::Error(format!(
                "Unable to convert {:?} to str",
                value
            )))),
        },
        None => None,
    }
}

/// Locate a udev block device by using its device node and apply a function
/// to it, returning the result of the function.
/// This approach is necessitated by the libudev lifetimes, which do not
/// allow returning anything directly obtained from the enumerator value
/// created in the method itself.
pub fn udev_block_device_apply<F, U>(devnode: &Path, f: F) -> StratisResult<Option<U>>
where
    F: FnOnce(&libudev::Device) -> U,
{
    #![allow(let_and_return)]
    let context = get_udev();

    let mut enumerator = libudev::Enumerator::new(context)?;
    enumerator.match_subsystem("block")?;

    let devnode = devnode.canonicalize()?;
    let result = match enumerator
        .scan_devices()?
        .find(|x| x.devnode().map_or(false, |d| devnode == d))
    {
        Some(device) => Ok(Some(f(&device))),
        None => Ok(None),
    };
    result
}

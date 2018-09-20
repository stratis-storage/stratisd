// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Udev dependent code for getting information about devices.

use std::ffi::OsStr;
use std::path::Path;

use libudev;

use stratis::{StratisError, StratisResult};

use super::super::super::udev::get_udev;

/// Returns true if udev indicates that the device belongs to Stratis, else
/// false. If there is any ambiguity, returns false.
/// This may happen if there was a failure to interpret a udev value that
/// was set.
pub fn stratis_device(device: &libudev::Device) -> bool {
    get_udev_property(device, "ID_FS_TYPE")
        .map_or(false, |v| v.map(|v| v == "stratis").unwrap_or(false))
}

/// Returns true if udev indicates that the device is a multipath member
/// device, else false. If there is any ambiguity, return true.
/// This may happen if there was a failure to interpret a udev value that
/// was set.
/// WARNING: This method can be relied on only if DM_MULTIPATH* udev
/// properties have been properly set by the time this property is read.
pub fn multipath_member(device: &libudev::Device) -> bool {
    get_udev_property(device, "DM_MULTIPATH_DEVICE_PATH")
        .map_or(false, |v| v.map(|v| v == "1").unwrap_or(true))
}

/// Devices that _must_ be ignored. Such a device may have Stratis metadata
/// on, but must _not_  be incorporated into stratisd hierarchy. The only
/// example of such a thing right now is a metadata member device.
/// WARNING: This method can be relied on only if DM_MULTIPATH* udev
/// properties have been properly set by the time this property is read.
pub fn must_ignore(device: &libudev::Device) -> bool {
    multipath_member(device)
}

/// If the expression is true, then it seems that no other system is
/// known to udev to claim this device.
/// Note from mulhern: I have no idea myself why this particular expression
/// should be correct. The expression is equivalent to that used in PR:
/// https://github.com/stratis-storage/stratisd/pull/936.
/// WARNING: This method can be relied on only if ID_* udev
/// properties have been properly set by the time they are read.
pub fn unclaimed(device: &libudev::Device) -> bool {
    (get_udev_property(device, "ID_PART_TABLE_TYPE").is_none()
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

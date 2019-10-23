// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! udev-related methods
use std::{collections::HashMap, ffi::OsStr, fs, path::Path};

use libudev;

use crate::stratis::{StratisError, StratisResult};

/// Make an enumerator for enumerating block devices. Return an error if there
/// was any udev-related error.
pub fn block_enumerator(context: &libudev::Context) -> libudev::Result<libudev::Enumerator> {
    let mut enumerator = libudev::Enumerator::new(context)?;
    enumerator.match_subsystem("block")?;
    Ok(enumerator)
}

/// Make an enumerator for enumerating stratis block devices. Return an error
/// if there was any udev-related error.
pub fn stratis_enumerator(context: &libudev::Context) -> libudev::Result<libudev::Enumerator> {
    let mut enumerator = libudev::Enumerator::new(context)?;
    enumerator.match_subsystem("block")?;
    enumerator.match_property("ID_FS_TYPE", "stratis")?;
    Ok(enumerator)
}

/// Get a udev property with the given name for the given device.
/// Returns None if no udev property found for the given property name.
/// Returns an error if the value of the property can not be converted to
/// a string using the standard conversion for this OS.
fn get_udev_property<T: AsRef<OsStr>>(
    device: &libudev::Device,
    property_name: T,
) -> Option<StratisResult<String>>
where
    T: std::fmt::Display,
{
    device
        .property_value(&property_name)
        .map(|value| match value.to_str() {
            Some(value) => Ok(value.into()),
            None => Err(StratisError::Error(format!(
                "Unable to convert udev property value with key {} belonging to device {} to a string",
                property_name,
                device.devnode().map_or("<unknown>".into(), |x| x.to_string_lossy().into_owned())
            ))),
        })
}

/// Returns true if udev indicates that the device is a multipath member
/// device, else false. Returns an error on a failure to interpret the
/// value.
pub fn is_multipath_member(device: &libudev::Device) -> StratisResult<bool> {
    match get_udev_property(device, "DM_MULTIPATH_DEVICE_PATH") {
        None => Ok(false),
        Some(Ok(value)) => Ok(value == "1"),
        Some(Err(err)) => Err(err),
    }
}

/// If the expression is true, then it seems that no other system is
/// known to udev to claim this device.
/// Note from mulhern: I have no idea myself why this particular expression
/// should be correct. I was told that the original source was dlehman.
pub fn is_unclaimed(device: &libudev::Device) -> bool {
    (get_udev_property(device, "ID_PART_TABLE_TYPE").is_none()
        || get_udev_property(device, "ID_PART_ENTRY_DISK").is_some())
        && get_udev_property(device, "ID_FS_USAGE").is_none()
}

/// Takes a libudev device entry and returns the properties as a HashMap.
fn device_as_map(device: &libudev::Device) -> HashMap<String, String> {
    let rc: HashMap<_, _> = device
        .properties()
        .map(|i| {
            (
                String::from(i.name().to_string_lossy()),
                String::from(i.value().to_string_lossy()),
            )
        })
        .collect();
    rc
}

/// Common function used to retrieve the udev db entry for a block device as a HashMap when found
pub fn get_udev_block_device(
    dev_node_search: &Path,
) -> StratisResult<Option<HashMap<String, String>>> {
    let context = libudev::Context::new()?;
    let mut enumerator = block_enumerator(&context)?;

    // Get canonical form to ensure we do correct lookup in udev db
    let canonical = fs::canonicalize(dev_node_search)?;

    let result = enumerator
        .scan_devices()?
        .filter(|dev| dev.is_initialized())
        .find(|x| x.devnode().map_or(false, |d| canonical == d))
        .map(|dev| device_as_map(&dev));
    Ok(result)
}

/// Lookup the WWN from the udev db using the device node eg. /dev/sda
pub fn hw_lookup(dev_node_search: &Path) -> StratisResult<Option<String>> {
    let dev = get_udev_block_device(dev_node_search)?;
    Ok(dev.and_then(|dev| dev.get("ID_WWN").cloned()))
}

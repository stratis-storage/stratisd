// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! udev-related methods
use std::{
    collections::HashMap,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use libudev;

use crate::stratis::{StratisError, StratisResult};

/// Get a udev property with the given name for the given device.
/// Returns None if no udev property found for the given property name.
/// Returns an error if the value of the property can not be converted to
/// a string using the standard conversion for this OS.
pub fn get_udev_property<T: AsRef<OsStr>>(
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

/// Make an enumerator for enumerating block devices
fn block_enumerator(context: &libudev::Context) -> StratisResult<libudev::Enumerator> {
    let mut enumerator = libudev::Enumerator::new(context)?;
    enumerator.match_subsystem("block")?;
    Ok(enumerator)
}

/// Make an enumerator for enumerating only Stratis block devices
pub fn stratis_enumerator(context: &libudev::Context) -> StratisResult<libudev::Enumerator> {
    let mut enumerator = libudev::Enumerator::new(context)?;
    enumerator.match_subsystem("block")?;
    enumerator.match_property("ID_FS_TYPE", "stratis")?;
    Ok(enumerator)
}

/// Locate a udev block device with the specified devnode and apply a function
/// to that device, returning the result.
/// Treat an uninitialized device as if it does not exist.
/// This approach is necessitated by the libudev lifetimes, which do not allow
/// returning anything directly obtained from the enumerator value created in
/// the method itself.
pub fn udev_block_device_apply<F, U>(devnode: &Path, f: F) -> StratisResult<Option<U>>
where
    F: FnOnce(&libudev::Device) -> U,
{
    let context = libudev::Context::new()?;
    let mut enumerator = block_enumerator(&context)?;

    let canonical = fs::canonicalize(devnode)?;

    Ok(enumerator
        .scan_devices()?
        .filter(|x| x.is_initialized())
        .find(|x| x.devnode().map_or(false, |d| canonical == d))
        .map(|d| f(&d)))
}

/// Takes a libudev device entry and returns the properties as a HashMap.
pub fn device_as_map(device: &libudev::Device) -> HashMap<String, String> {
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

/// Collect paths for all the block devices which are not individual multipath paths and which
/// appear to be empty from a udev perspective.
pub fn get_all_empty_devices() -> StratisResult<Vec<PathBuf>> {
    let context = libudev::Context::new()?;
    let mut enumerator = block_enumerator(&context)?;

    Ok(enumerator
        .scan_devices()?
        .filter(|dev| dev.is_initialized())
        .filter(|dev| {
            dev.property_value("DM_MULTIPATH_DEVICE_PATH")
                .map_or(true, |v| v != "1")
                && !((dev.property_value("ID_PART_TABLE_TYPE").is_some()
                    && dev.property_value("ID_PART_ENTRY_DISK").is_none())
                    || dev.property_value("ID_FS_USAGE").is_some())
        })
        .filter_map(|i| i.devnode().map(|d| d.into()))
        .collect())
}

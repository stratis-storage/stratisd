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

use devicemapper::Device;

use crate::stratis::{StratisError, StratisResult};

/// Returns true if udev indicates that the device belongs to Stratis, else
/// false. If there is any ambiguity, returns false.
/// This may happen if there was a failure to interpret a udev value that
/// was set.
fn stratis_device(device: &libudev::Device) -> bool {
    get_udev_property(device, "ID_FS_TYPE")
        .map_or(false, |v| v.map(|v| v == "stratis").unwrap_or(false))
}

/// Returns true if udev indicates that the device is a multipath member
/// device, else false. If there is any ambiguity, return true.
/// This may happen if there was a failure to interpret a udev value that
/// was set.
/// WARNING: This method can be relied on only if DM_MULTIPATH* udev
/// properties have been properly set by the time this property is read.
fn multipath_member(device: &libudev::Device) -> bool {
    get_udev_property(device, "DM_MULTIPATH_DEVICE_PATH")
        .map_or(false, |v| v.map(|v| v == "1").unwrap_or(true))
}

/// Devices that _must_ be ignored. Such a device may have Stratis metadata
/// on, but must _not_  be incorporated into stratisd hierarchy. The only
/// example of such a thing right now is a metadata member device.
/// WARNING: This method can be relied on only if DM_MULTIPATH* udev
/// properties have been properly set by the time this property is read.
#[allow(dead_code)]
fn must_ignore(device: &libudev::Device) -> bool {
    multipath_member(device)
}

/// Categories for devices.
pub enum Ownership {
    /// A member of a multipath device.
    MultipathMember,
    /// A Stratis device.
    StratisDevice,
}

/// Identify a device based solely on its udev information.
#[allow(dead_code)]
pub fn identify(device: &libudev::Device) -> Option<Ownership> {
    if multipath_member(device) {
        Some(Ownership::MultipathMember)
    } else if stratis_device(device) {
        Some(Ownership::StratisDevice)
    } else {
        None
    }
}

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

/// Locate a udev block device with the specified devnode and apply a function
/// to that device, returning the result.
/// This approach is necessitated by the libudev lifetimes, which do not allow
/// returning anything directly obtained from the enumerator value created in
/// the method itself.
pub fn udev_block_device_apply<F, U>(devnode: &Path, f: F) -> StratisResult<Option<U>>
where
    F: FnOnce(&libudev::Device) -> U,
{
    let context = libudev::Context::new()?;
    let mut enumerator = libudev::Enumerator::new(&context)?;
    enumerator.match_subsystem("block")?;

    Ok(enumerator
        .scan_devices()?
        .find(|x| x.devnode().map_or(false, |d| devnode == d))
        .map(|d| f(&d)))
}

/// Obtain a devicemapper Device and the devnode from a libudev Device.
/// Returns None if the device node or the device number, used to construct
/// the devicemapper device, can not be otained from libudev.
#[allow(dead_code)]
pub fn get_device_and_devnode(device: &libudev::Device) -> Option<(Device, PathBuf)> {
    device.devnode().and_then(|devnode| {
        device
            .devnum()
            .and_then(|devnum| Some((Device::from(devnum), PathBuf::from(devnode))))
    })
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

/// Retrieve all the block devices on the system that have a Stratis signature.
pub fn get_stratis_block_devices() -> StratisResult<Vec<PathBuf>> {
    let context = libudev::Context::new()?;
    let mut enumerator = libudev::Enumerator::new(&context)?;
    enumerator.match_subsystem("block")?;
    enumerator.match_property("ID_FS_TYPE", "stratis")?;

    Ok(enumerator
        .scan_devices()?
        .filter(|dev| dev.is_initialized())
        .filter(|dev| !multipath_member(dev))
        .filter_map(|i| i.devnode().map(|d| d.into()))
        .collect())
}

/// Common function used to retrieve the udev db entry for a block device as a HashMap when found
pub fn get_udev_block_device(
    dev_node_search: &Path,
) -> StratisResult<Option<HashMap<String, String>>> {
    let context = libudev::Context::new()?;
    let mut enumerator = libudev::Enumerator::new(&context)?;
    enumerator.match_subsystem("block")?;

    // Get canonical form to ensure we do correct lookup in udev db
    let canonical = fs::canonicalize(dev_node_search)?;

    let result = enumerator
        .scan_devices()?
        .filter(|dev| dev.is_initialized())
        .find(|x| x.devnode().map_or(false, |d| canonical == d))
        .map(|dev| device_as_map(&dev));
    Ok(result)
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! udev-related methods
use std::{ffi::OsStr, path::Path};

use libudev;

use crate::stratis::{StratisError, StratisResult};

/// Make an enumerator for enumerating block devices. Return an error if there
/// was any udev-related error.
pub fn block_enumerator(context: &libudev::Context) -> libudev::Result<libudev::Enumerator> {
    let mut enumerator = libudev::Enumerator::new(context)?;
    enumerator.match_subsystem("block")?;
    Ok(enumerator)
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
fn is_unclaimed(device: &libudev::Device) -> bool {
    (get_udev_property(device, "ID_PART_TABLE_TYPE").is_none()
        || get_udev_property(device, "ID_PART_ENTRY_DISK").is_some())
        && get_udev_property(device, "ID_FS_USAGE").is_none()
}

/// Return true if the device is identified by udev as belonging to Stratis.
/// Return an error if a udev property value could not be converted.
fn is_stratis(device: &libudev::Device) -> StratisResult<bool> {
    match get_udev_property(device, "ID_FS_TYPE") {
        None => Ok(false),
        Some(Ok(value)) => Ok(value == "stratis"),
        Some(Err(err)) => Err(err),
    }
}

/// An enum to encode udev classification of a device
#[derive(Debug, Eq, PartialEq)]
pub enum UdevOwnership {
    MultipathMember,
    Stratis,
    Theirs,
    Unowned,
}

/// Decide the ownership of a device based on udev information.
///
/// Always check multipath member status first. The theory is that a multipath
/// member may also appear to belong to Stratis, but it should not be identified
/// as a Stratis device.
///
/// Note that the designation Theirs, i.e., the device already belongs to some
/// other entity is the default designation. This seems counterintuitive, but
/// it is the unclaimed designation that has a boolean expression on udev
/// properties associated with it.
pub fn decide_ownership(device: &libudev::Device) -> StratisResult<UdevOwnership> {
    // We believe that it is possible to be a multipath member and also to
    // be identified as a Stratis device. The designations are not mutually
    // exclusive, but the multipath member device must not be used by Stratis.
    if is_multipath_member(device)? {
        return Ok(UdevOwnership::MultipathMember);
    }

    // We believe that the following designations are mutually exclusive, i.e.
    // it is not possible to be a Stratis device and also to appear unowned.
    Ok(if is_stratis(device)? {
        UdevOwnership::Stratis
    } else if is_unclaimed(device) {
        UdevOwnership::Unowned
    } else {
        UdevOwnership::Theirs
    })
}

/// Locate a udev block device with the specified devnode and apply a function
/// to that device, returning the result.
/// This approach is necessitated by the libudev lifetimes, which do not allow
/// returning anything directly obtained from the enumerator value created in
/// the method itself.
/// Note that this does require iterating through the blockdevs in the udev
/// database, so it is essentially linear in the number of block devices.
pub fn block_device_apply<F, U>(devnode: &Path, f: F) -> StratisResult<Option<U>>
where
    F: FnOnce(&libudev::Device) -> U,
{
    let canonical = devnode.canonicalize()?;

    let context = libudev::Context::new()?;
    let mut enumerator = block_enumerator(&context)?;

    Ok(enumerator
        .scan_devices()?
        .filter(|dev| dev.is_initialized())
        .find(|x| x.devnode().map_or(false, |d| canonical == d))
        .map(|d| f(&d)))
}

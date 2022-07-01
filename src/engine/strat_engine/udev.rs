// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! udev-related methods
use std::{ffi::OsStr, fmt};

use crate::{
    engine::types::{DevicePath, UdevEngineDevice},
    stratis::{StratisError, StratisResult},
};

/// Key for a udev property
pub const FS_TYPE_KEY: &str = "ID_FS_TYPE";

/// Possible values for the "ID_FS_TYPE" udev property
pub const STRATIS_FS_TYPE: &str = "stratis";
pub const CRYPTO_FS_TYPE: &str = "crypto_LUKS";

/// Possible value for a udev subsystem designation
pub const SUBSYSTEM_BLOCK: &str = "block";

/// Make an enumerator for enumerating block devices. Return an error if there
/// was any udev-related error.
pub fn block_enumerator(context: &libudev::Context) -> libudev::Result<libudev::Enumerator<'_>> {
    let mut enumerator = libudev::Enumerator::new(context)?;
    enumerator.match_subsystem(SUBSYSTEM_BLOCK)?;
    Ok(enumerator)
}

/// Get a udev property with the given name for the given device.
/// Returns None if no udev property found for the given property name.
/// Returns an error if the value of the property can not be converted to
/// a string using the standard conversion for this OS.
pub fn get_udev_property<T: AsRef<OsStr> + fmt::Display>(
    device: &UdevEngineDevice,
    property_name: T,
) -> Option<StratisResult<String>> {
    device
        .property_value(&property_name)
        .map(|opt| opt.map(|s| s.to_string()))
}

/// Returns true if udev indicates that the device is a multipath member
/// device, else false. Returns an error on a failure to interpret the
/// value.
fn is_multipath_member(device: &UdevEngineDevice) -> StratisResult<bool> {
    match get_udev_property(device, "DM_MULTIPATH_DEVICE_PATH") {
        None => Ok(false),
        Some(Ok(value)) => Ok(value == "1"),
        Some(Err(err)) => Err(err),
    }
}

/// If the expression is true, then it seems that no other system is
/// known to udev to claim this device.
fn is_unclaimed(device: &UdevEngineDevice) -> bool {
    (get_udev_property(device, "ID_PART_TABLE_TYPE").is_none()
        || get_udev_property(device, "ID_PART_ENTRY_DISK").is_some())
        && get_udev_property(device, "ID_FS_USAGE").is_none()
}

/// Return true if the device is identified by udev as belonging to Stratis.
/// Return an error if a udev property value could not be converted.
fn is_stratis(device: &UdevEngineDevice) -> StratisResult<bool> {
    match get_udev_property(device, FS_TYPE_KEY) {
        None => Ok(false),
        Some(Ok(value)) => Ok(value == STRATIS_FS_TYPE),
        Some(Err(err)) => Err(err),
    }
}

/// Return true if the device is identified by udev as being an encrypted
/// LUKS device. Return an error if a udev property could not be converted.
fn is_luks(device: &UdevEngineDevice) -> StratisResult<bool> {
    match get_udev_property(device, FS_TYPE_KEY) {
        None => Ok(false),
        Some(Ok(value)) => Ok(value == CRYPTO_FS_TYPE),
        Some(Err(err)) => Err(err),
    }
}

/// An enum to encode udev classification of a device
#[derive(Debug, Eq, PartialEq)]
pub enum UdevOwnership {
    Luks,
    MultipathMember,
    Stratis,
    Theirs,
    Unowned,
}

impl fmt::Display for UdevOwnership {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UdevOwnership::Luks => write!(f, "LUKS encrypted block device"),
            UdevOwnership::MultipathMember => write!(f, "member of a multipath block device"),
            UdevOwnership::Stratis => write!(f, "Stratis block device"),
            UdevOwnership::Theirs => write!(f, "block device which appears to be owned"),
            UdevOwnership::Unowned => write!(f, "block device which appears to be unowned"),
        }
    }
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
pub fn decide_ownership(device: &UdevEngineDevice) -> StratisResult<UdevOwnership> {
    || -> StratisResult<UdevOwnership> {
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
        } else if is_luks(device)? {
            UdevOwnership::Luks
        } else if is_unclaimed(device) {
            UdevOwnership::Unowned
        } else {
            UdevOwnership::Theirs
        })
    }()
    .map_err(|err| {
        StratisError::Chained(
            "Could not determine ownership of a device from a udev database entry".to_string(),
            Box::new(err),
        )
    })
}

/// Locate a udev block device with the specified devnode and apply a function
/// to that device, returning the result.
/// Note that this does require iterating through the blockdevs in the udev
/// database, so it is essentially linear in the number of block devices.
/// This approach was initially required because udev lifetimes did not allow
/// the object representing the device to be returned from a method. This is
/// now unnecessary; the method could simply return a UdevEngineDevice.
/// It was decided not to take that step as, at some point in the future, it
/// might be reasonable to revert to an approach which reads the udev device
/// object and applies a function only to selected properties or attributes.
pub fn block_device_apply<F, U>(device_path: &DevicePath, f: F) -> StratisResult<Option<U>>
where
    F: FnOnce(&UdevEngineDevice) -> U,
{
    let context = libudev::Context::new()?;
    let mut enumerator = block_enumerator(&context)?;

    Ok(enumerator
        .scan_devices()?
        .filter(|dev| dev.is_initialized())
        .find(|x| x.devnode().map_or(false, |d| **device_path == *d))
        .map(|ref d| f(&UdevEngineDevice::from(d))))
}

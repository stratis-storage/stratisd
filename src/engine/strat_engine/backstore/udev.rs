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

use crate::{
    engine::strat_engine::backstore::identify,
    stratis::{StratisError, StratisResult},
};

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

/// If the expression is true, then it seems that no other system is
/// known to udev to claim this device.
/// Note from mulhern: I have no idea myself why this particular expression
/// should be correct. I was told that the original source was dlehman.
/// WARNING: This method can be relied on only if ID_* udev
/// properties have been properly set by the time they are read.
fn unclaimed(device: &libudev::Device) -> bool {
    (get_udev_property(device, "ID_PART_TABLE_TYPE").is_none()
        || get_udev_property(device, "ID_PART_ENTRY_DISK").is_some())
        && get_udev_property(device, "ID_FS_USAGE").is_none()
}

/// Categories for devices.
pub enum UdevOwnership {
    /// A member of a multipath device.
    MultipathMember,
    /// A Stratis device.
    StratisDevice,
    /// Apparently unowned.
    Unowned,
    /// Not apparently unowned, so presumably owned by another.
    /// Note that this is the default case, when all others are eliminated.
    OwnedByOther,
}

/// Identify a device based solely on its udev information.
#[allow(dead_code)]
pub fn identify(device: &libudev::Device) -> UdevOwnership {
    if multipath_member(device) {
        UdevOwnership::MultipathMember
    } else if stratis_device(device) {
        UdevOwnership::StratisDevice
    } else if unclaimed(device) {
        UdevOwnership::Unowned
    } else {
        UdevOwnership::OwnedByOther
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

/// Collect paths for all the block devices which are not individual multipath paths and which
/// appear to be empty from a udev perspective.
fn get_all_empty_devices() -> StratisResult<Vec<PathBuf>> {
    let context = libudev::Context::new()?;
    let mut enumerator = libudev::Enumerator::new(&context)?;
    enumerator.match_subsystem("block")?;

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

/// Retrieve all the block devices on the system that have a Stratis signature.
pub fn get_stratis_block_devices() -> StratisResult<Vec<PathBuf>> {
    let context = libudev::Context::new()?;
    let mut enumerator = libudev::Enumerator::new(&context)?;
    enumerator.match_subsystem("block")?;
    enumerator.match_property("ID_FS_TYPE", "stratis")?;

    let devices: Vec<PathBuf> = enumerator
        .scan_devices()?
        .filter(|dev| dev.is_initialized())
        .filter(|dev| {
            dev.property_value("DM_MULTIPATH_DEVICE_PATH")
                .map_or(true, |v| v != "1")
        })
        .filter_map(|i| i.devnode().map(|d| d.into()))
        .collect();

    if devices.is_empty() {
        // We have found no Stratis devices, possible reasons are:
        // 1. We really don't have any
        // 2. We have some, but libblkid is too old to support Stratis, thus we appear empty
        // 3. We ran this code at early boot before we have any udev db entries which are complete
        //    or are complete but fall into reasons 1 & 2 above
        //
        // In this case we will get all the block devices which have complete udev db block device
        // entries and appear "empty" and go out to disk and check them!

        Ok(get_all_empty_devices()?
            .into_iter()
            .filter(|x| {
                identify(x)
                    .map(|ownership| ownership.stratis_identifiers().is_some())
                    .unwrap_or(false)
            })
            .collect())
    } else {
        Ok(devices)
    }
}

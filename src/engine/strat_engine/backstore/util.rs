// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Utilities to support Stratis.
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;

use libudev;

use stratis::{ErrorEnum, StratisError, StratisResult};

/// Takes a libudev device entry and returns the properties as a HashMap.
/// Omits any properties with names or values that can not be translated from
/// the internal encoding.
fn device_as_map(device: &libudev::Device) -> HashMap<String, String> {
    device
        .properties()
        .map(|i| {
            (
                i.name().to_str().and_then(|n| Some(n.to_string())),
                i.value().to_str().and_then(|v| Some(v.to_string())),
            )
        })
        .filter(|&(ref n, ref v)| n.is_some() && v.is_some())
        .map(|(n, v)| (n.expect("!n.is_none()"), v.expect("!v.is_none()")))
        .collect()
}

/// Common function used to retrieve the udev db entry for a block device as a HashMap when found
pub fn get_udev_block_device(
    dev_node_search: &Path,
) -> StratisResult<Option<HashMap<String, String>>> {
    let context = libudev::Context::new()?;
    let mut enumerator = libudev::Enumerator::new(&context)?;
    enumerator.match_subsystem("block")?;

    let result = enumerator
        .scan_devices()?
        .find(|x| x.devnode().map_or(false, |d| dev_node_search == d))
        .and_then(|dev| Some(device_as_map(&dev)));
    Ok(result)
}

#[allow(dead_code)]
/// Get a udev property.
/// Return an error if the property value can not be converted to a String.
pub fn get_udev_property<T: AsRef<OsStr>>(
    device: &libudev::Device,
    property_name: T,
) -> StratisResult<Option<String>> {
    match device.property_value(property_name) {
        Some(value) => match value.to_str() {
            Some(value) => Ok(Some(value.into())),
            None => Err(StratisError::Engine(
                ErrorEnum::Error,
                format!("Unable to convert {:?} to str", value),
            )),
        },
        None => Ok(None),
    }
}

/// Lookup the WWN from the udev db using the device node eg. /dev/sda
pub fn hw_lookup(dev_node_search: &Path) -> StratisResult<Option<String>> {
    let dev = get_udev_block_device(dev_node_search)?;
    Ok(dev.and_then(|dev| dev.get("ID_WWN").and_then(|i| Some(i.clone()))))
}

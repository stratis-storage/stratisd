// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Utilities to support Stratis.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use libudev;

use super::device::is_stratis_device;
use stratis::StratisResult;

/// Takes a libudev device entry and returns the properties as a HashMap.
fn device_as_map(device: &libudev::Device) -> HashMap<String, String> {
    device
        .properties()
        .map(|i| {
            (
                String::from(i.name().to_str().expect("Unix is utf-8")),
                String::from(i.value().to_str().expect("Unix is utf-8")),
            )
        })
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
        .map_or(None, |dev| Some(device_as_map(&dev)));
    Ok(result)
}

/// Lookup the WWN from the udev db using the device node eg. /dev/sda
pub fn hw_lookup(dev_node_search: &Path) -> StratisResult<Option<String>> {
    let dev = get_udev_block_device(dev_node_search)?;
    Ok(dev.map_or(None, |dev| {
        dev.get("ID_WWN").map_or(None, |i| Some(i.clone()))
    }))
}

/// Collect paths for all the devices that appear to be empty from a udev db perspective.
fn get_all_empty_devices() -> StratisResult<Vec<PathBuf>> {
    let context = libudev::Context::new()?;
    let mut enumerator = libudev::Enumerator::new(&context)?;
    enumerator.match_subsystem("block")?;

    Ok(enumerator
        .scan_devices()?
        .filter(|dev| {
            !((dev.property_value("ID_PART_TABLE_TYPE").is_some()
                && !dev.property_value("ID_PART_ENTRY_DISK").is_some())
                || dev.property_value("ID_FS_USAGE").is_some())
        })
        .map(|i| i.devnode().expect("block devices have devnode").into())
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
        .map(|x| x.devnode().expect("block devices have devnode").into())
        .collect();

    if devices.is_empty() {
        // Either we don't have any stratis devices or we are using a distribution that doesn't
        // have a version of libblkid that supports stratis, lets make sure.
        // TODO: At some point in the future we can remove this and just return the devices.

        Ok(get_all_empty_devices()?
            .into_iter()
            .filter(|x| is_stratis_device(&x).ok().is_some())
            .collect())
    } else {
        Ok(devices)
    }
}

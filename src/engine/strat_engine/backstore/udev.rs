// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! udev-related methods
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use libudev;

use crate::{engine::strat_engine::backstore::device::identify, stratis::StratisResult};

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

/// Lookup the WWN from the udev db using the device node eg. /dev/sda
pub fn hw_lookup(dev_node_search: &Path) -> StratisResult<Option<String>> {
    let dev = get_udev_block_device(dev_node_search)?;
    Ok(dev.and_then(|dev| dev.get("ID_WWN").cloned()))
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

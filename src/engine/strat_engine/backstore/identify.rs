// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Discover or identify devices that may belong to Stratis.

use std::{
    collections::HashMap,
    fs::OpenOptions,
    path::{Path, PathBuf},
};

use libudev;

use devicemapper::Device;

use crate::engine::{
    strat_engine::{
        backstore::metadata::device_identifiers,
        udev::{block_enumerator, decide_ownership, UdevOwnership},
    },
    types::{DevUuid, PoolUuid},
};

// A wrapper for obtaining the device number as a devicemapper Device
// which interprets absence of the value as an error, which it is in this
// context.
fn device_to_devno_wrapper(device: &libudev::Device) -> Result<Device, String> {
    device
        .devnum()
        .ok_or_else(|| "udev entry did not contain a device number".into())
        .map(Device::from)
}

// A wrapper around the metadata module's device_identifers method
// which also handles failure to open a device for reading.
// Returns an error if the device could not be opened for reading.
// Returns Ok(Err(...)) if there was an error while reading the
// Stratis identifiers from the device.
// Returns Ok(Ok(None)) if the identifers did not appear to be on
// the device.
fn device_identifiers_wrapper(
    devnode: &Path,
) -> Result<Result<Option<(PoolUuid, DevUuid)>, String>, String> {
    OpenOptions::new()
        .read(true)
        .open(devnode)
        .as_mut()
        .map_err(|err| {
            format!(
                "device {} could not be opened for reading: {}",
                devnode.display(),
                err
            )
        })
        .map(|f| {
            device_identifiers(f).map_err(|err| {
                format!(
                    "encountered an error while reading Stratis header for device {}: {}",
                    devnode.display(),
                    err
                )
            })
        })
}

/// Process a device which udev information indicates is a Stratis device.
fn process_stratis_device(dev: &libudev::Device) -> Option<(PoolUuid, DevUuid, Device, PathBuf)> {
    match dev.devnode() {
        Some(devnode) => {
            match (
                device_to_devno_wrapper(dev),
                device_identifiers_wrapper(devnode),
            ) {
                (Err(err), _) | (_, Err(err)) | (_, Ok(Err(err))) => {
                    warn!("udev identified device {} as a Stratis device but {}, disregarding the device",
                          devnode.display(),
                          err);
                    None
                }
                (_, Ok(Ok(None))) => {
                    warn!("udev identified device {} as a Stratis device but there appeared to be no Stratis metadata on the device, disregarding the device",
                          devnode.display());
                    None
                }
                (Ok(devno), Ok(Ok(Some((pool_uuid, device_uuid))))) => {
                    Some((pool_uuid, device_uuid, devno, devnode.to_path_buf()))
                }
            }
        }
        None => {
            warn!("udev identified a device as a Stratis device, but the udev entry for the device had no device node, disregarding device");
            None
        }
    }
}

/// Process a device which udev information indicates is unowned.
fn process_unowned_device(dev: &libudev::Device) -> Option<(PoolUuid, DevUuid, Device, PathBuf)> {
    match dev.devnode() {
        Some(devnode) => {
            match (
                device_to_devno_wrapper(dev),
                device_identifiers_wrapper(devnode),
            ) {
                (Err(err), _) | (_, Err(err)) => {
                    warn!("udev identified device {} as a block device but {}, disregarding the device",
                          devnode.display(),
                          err);
                    None
                }
                // FIXME: Refine error return in StaticHeader::setup(),
                // so it can be used to distinguish between signficant
                // and insignficant errors and then use that ability to
                // distinguish here between different levels of
                // severity.
                (_, Ok(Err(err))) => {
                    debug!("udev identified device {} as a block device but {}, disregarding the device",
                           devnode.display(),
                           err);
                    None
                }
                (_, Ok(Ok(None))) => None,
                (Ok(devno), Ok(Ok(Some((pool_uuid, device_uuid))))) => {
                    Some((pool_uuid, device_uuid, devno, devnode.to_path_buf()))
                }
            }
        }
        None => {
            warn!("udev identified a device as a block device, but the udev entry for the device had no device node, disregarding the device");
            None
        }
    }
}

// Use udev to identify all block devices and return the subset of those
// that have Stratis signatures.
fn find_all_block_devices_with_stratis_signatures(
) -> libudev::Result<HashMap<PoolUuid, HashMap<Device, PathBuf>>> {
    let context = libudev::Context::new()?;
    let mut enumerator = block_enumerator(&context)?;

    let pool_map = enumerator
        .scan_devices()?
        .filter_map(|dev| identify_block_device(&dev))
        .fold(
            HashMap::new(),
            |mut acc, (pool_uuid, _, device, devnode)| {
                acc.entry(pool_uuid)
                    .or_insert_with(HashMap::new)
                    .insert(device, devnode);
                acc
            },
        );

    Ok(pool_map)
}

// Find all devices identified by udev as Stratis devices.
fn find_all_stratis_devices() -> libudev::Result<HashMap<PoolUuid, HashMap<Device, PathBuf>>> {
    let context = libudev::Context::new()?;
    let mut enumerator = block_enumerator(&context)?;
    enumerator.match_property("ID_FS_TYPE", "stratis")?;

    let pool_map = enumerator.scan_devices()?
        .filter(|dev| {
            let initialized = dev.is_initialized();
            if !initialized {
                warn!("Found a udev entry for a device identified as a Stratis device, but udev also identified it as uninitialized, omitting the device from the set of devices to process, for safety");
            };
            initialized
        })
        .filter_map(|dev| {
            decide_ownership(&dev)
                .map_err(|err| {
                    warn!("Could not determine ownership of a block device identified as a Stratis device by udev because of an error processing udev information, omitting the device from the set of devices to process, for safety: {}",
                          err);
                })
            .map(|decision| match decision {
                UdevOwnership::Stratis => process_stratis_device(&dev),
                UdevOwnership::MultipathMember => None,
                _ => {
                    warn!("udev enumeration identified this device as a Stratis block device but on further examination it appears not to belong to Stratis");
                    None
                }
            })
            .unwrap_or(None)
        })
        .fold(HashMap::new(), |mut acc, (pool_uuid, _, device, devnode)| {
            acc.entry(pool_uuid).or_insert_with(HashMap::new).insert(device, devnode);
            acc
        });
    Ok(pool_map)
}

/// Identify a block device in the context where a udev event has been
/// captured for some block device. Return None if the device does not
/// appear to be a Stratis device. Log at an appropriate level on all errors.
pub fn identify_block_device(
    dev: &libudev::Device,
) -> Option<(PoolUuid, DevUuid, Device, PathBuf)> {
    let initialized = dev.is_initialized();
    if !initialized {
        debug!("Found a udev entry for a device identified as a block device, but udev also identified it as uninitialized, disregarding the device");
        return None;
    };

    match decide_ownership(dev) {
        Err(err) => {
            warn!("Could not determine ownership of a udev block device because of an error processing udev information, disregarding the device: {}",
                  err);
            None
        }
        Ok(ownership) => match ownership {
            UdevOwnership::Stratis => process_stratis_device(dev),
            UdevOwnership::Unowned => process_unowned_device(dev),
            _ => None,
        },
    }
}

/// Retrieve all block devices that should be made use of by the
/// Stratis engine. This excludes Stratis block devices that appear to be
/// multipath members.
///
/// Includes a fallback path, which is used if no Stratis block devices are
/// found using the obvious udev property- and enumerator-based approach.
/// This fallback path is more expensive, because it must search all block
/// devices via udev rather than just all Stratis block devices.
///
/// Omits any device that appears problematic in some way.
///
/// Return an error only on a failure to construct or scan with a udev
/// enumerator.
///
/// Returns a map of pool uuids to a map of devices to devnodes for each pool.
pub fn find_all() -> libudev::Result<HashMap<PoolUuid, HashMap<Device, PathBuf>>> {
    info!("Beginning initial search for Stratis block devices");
    let pool_map = find_all_stratis_devices()?;

    if pool_map.is_empty() {
        // No Stratis devices have been found, possible reasons are:
        // 1. There are none
        // 2. There are some but libblkid version is less than 2.32, so
        // Stratis devices are not recognized by udev.
        // 3. There are many incomplete udev entries, because this code is
        // being run before the udev database is populated.
        //
        // Try again to find Stratis block devices, but this time enumerate
        // all block devices, not just all the ones that can be identified
        // as Stratis blockdevs by udev, and process only those that appear
        // unclaimed or appear to be claimed by Stratis (and not
        // multipath members).

        info!("Could not identify any Stratis devices by a udev search for devices with ID_FS_TYPE=\"stratis\"; using fallback search mechanism");

        find_all_block_devices_with_stratis_signatures()
    } else {
        Ok(pool_map)
    }
}

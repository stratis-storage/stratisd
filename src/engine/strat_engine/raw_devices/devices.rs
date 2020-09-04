// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::{
    collections::{HashMap, HashSet},
    fs::OpenOptions,
    path::{Path, PathBuf},
};

use itertools::Itertools;

use devicemapper::{Bytes, Device, IEC};

use crate::{
    engine::{
        strat_engine::{
            device::blkdev_size,
            metadata::{device_identifiers, StratisIdentifiers},
            raw_devices::udev::{
                block_device_apply, decide_ownership, get_udev_property, UdevOwnership,
            },
        },
        types::{DevUuid, PoolUuid},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

const MIN_DEV_SIZE: Bytes = Bytes(IEC::Gi);

// Get information that can be obtained from udev for the block device
// identified by devnode. Return an error if there was an error finding the
// information or no udev entry corresponding to the devnode could be found.
// Return an error if udev ownership could not be obtained.
fn udev_info(
    devnode: &Path,
) -> StratisResult<(UdevOwnership, Device, Option<StratisResult<String>>)> {
    block_device_apply(devnode, |d| {
        (
            decide_ownership(d),
            d.devnum(),
            get_udev_property(d, "ID_WWN"),
        )
    })
    .and_then(|res| {
        res.ok_or_else(|| {
            StratisError::Engine(
                ErrorEnum::NotFound,
                format!(
                    "Block device {} could not be found in the udev database",
                    devnode.display()
                ),
            )
        })
    })
    .map_err(|err| {
        StratisError::Engine(
            ErrorEnum::NotFound,
            format!(
                "Could not obtain udev information for block device {}: {}",
                devnode.display(),
                err
            ),
        )
    })
    .and_then(|(ownership, devnum, id_wwn)| {
        devnum
            .ok_or_else(|| {
                StratisError::Error(format!(
                    "Insufficient information: no device number found for device {} using udev",
                    devnode.display()
                ))
            })
            .map(|dev| (ownership, Device::from(dev), id_wwn))
    })
    .and_then(|(ownership, devnum, id_wwn)| {
        ownership
            .map(|ownership| (ownership, devnum, id_wwn))
            .map_err(|err| {
                StratisError::Error(format!(
                    "Could not obtain ownership information for device {} using udev: {}",
                    devnode.display(),
                    err
                ))
            })
    })
}

// Find information from the devnode that is useful to identify a device or
// to construct a StratBlockDev object. Returns a tuple of the ID_WWN,
// the size of the device, and Stratis identifiers for the device, if any
// are found. If the value for the Stratis identifiers is None, then this
// device has been determined to be unowned.
#[allow(clippy::type_complexity)]
fn dev_info(
    devnode: &Path,
) -> StratisResult<(
    Option<StratisResult<String>>,
    Bytes,
    Option<StratisIdentifiers>,
    Device,
)> {
    let (ownership, devnum, hw_id) = udev_info(devnode)?;
    match ownership {
        UdevOwnership::Luks | UdevOwnership::MultipathMember | UdevOwnership::Theirs => {
            let err_str = format!(
                "udev information indicates that device {} is a {}",
                devnode.display(),
                ownership
            );
            Err(StratisError::Engine(ErrorEnum::Invalid, err_str))
        }
        UdevOwnership::Stratis | UdevOwnership::Unowned => {
            let mut f = OpenOptions::new().read(true).write(true).open(&devnode)?;
            let dev_size = blkdev_size(&f)?;

            // FIXME: Read device identifiers from either an Unowned or a
            // Stratis device. For a Stratis device, this is the correct thing
            // to do. For an unowned device, this is the best available check
            // that we currently have to prevent overwriting a device which
            // is owned, but which udev has not identified as such. In future,
            // we hope to use libblkid in order to double check that the
            // device is truly unowned, not just for Stratis but also for
            // other potential owners.
            let stratis_identifiers = device_identifiers(&mut f).map_err(|err| {
                let error_message = format!(
                    "There was an error reading Stratis metadata from device {}; the device is unsuitable for initialization: {}",
                    devnode.display(),
                    err
                );
                StratisError::Engine(ErrorEnum::Invalid, error_message)
            })?;

            if ownership == UdevOwnership::Stratis && stratis_identifiers.is_none() {
                let error_message = format!(
                    "udev identified device {} as a Stratis device but device metadata does not show that it is a Stratis device",
                    devnode.display()
                );
                return Err(StratisError::Engine(ErrorEnum::Invalid, error_message));
            }

            Ok((hw_id, dev_size, stratis_identifiers, devnum))
        }
    }
}

/// A miscellaneous grab bag of useful information required to decide whether
/// a device should be allowed to be initialized by Stratis or to be used
/// when initializing a device.
#[derive(Debug)]
pub struct InitDeviceInfo {
    /// The device number
    pub devno: Device,
    /// The devnode
    pub devnode: PathBuf,
    /// The ID_WWN udev value, if present, supposed to uniquely identify the
    /// device.
    pub id_wwn: Option<StratisResult<String>>,
    /// The total size of the device
    pub size: Bytes,
}

// Process a list of devices specified as device nodes.
//
// * Reduce the list of devices to a set.
// * Return a vector of accumulated information about the device nodes.
//
// If the StratisIdentifiers value is not None, then the device has been
// identified as a Stratis device.
//
// Return an error if there was an error collecting the information or
// if it turns out that at least two of the specified devices have the same
// device number.
fn process_devices(
    paths: &[&Path],
) -> StratisResult<Vec<(InitDeviceInfo, Option<StratisIdentifiers>)>> {
    let infos = paths
        .iter()
        .unique()
        .map(|devnode| {
            dev_info(devnode).map(|(id_wwn, size, stratis_identifiers, devno)| {
                (
                    InitDeviceInfo {
                        devno,
                        devnode: devnode.to_path_buf(),
                        id_wwn,
                        size,
                    },
                    stratis_identifiers,
                )
            })
        })
        .collect::<StratisResult<Vec<(InitDeviceInfo, Option<StratisIdentifiers>)>>>()
        .map_err(|err| {
            let error_message = format!(
                "At least one of the devices specified was unsuitable for initialization: {}",
                err
            );
            StratisError::Engine(ErrorEnum::Invalid, error_message)
        })?;

    let duplicate_device_number_messages: Vec<String> = infos
        .iter()
        .map(|(info, _)| (info.devno, info.devnode.to_path_buf()))
        .fold(HashMap::new(), |mut acc, (devno, devnode)| {
            acc.entry(devno).or_insert_with(Vec::new).push(devnode);
            acc
        })
        .iter()
        .filter(|(_, devnodes)| devnodes.len() > 1)
        .map(|(devno, devnodes)| {
            format!(
                "device nodes {} correspond to device number {}",
                devnodes.iter().map(|d| d.display()).join(", "),
                devno
            )
        })
        .collect();

    if !duplicate_device_number_messages.is_empty() {
        return Err(StratisError::Engine(
            ErrorEnum::Invalid,
            format!(
                "At least two of the devices specified have the same device number: {}",
                duplicate_device_number_messages.join("; ")
            ),
        ));
    }

    Ok(infos)
}

// Check coherence of pool and device UUIDs against a set of current UUIDs.
// If the selection of devices is incompatible with the current
// state of the set, or simply invalid, return an error.
//
// Postcondition: There are no infos returned from this method that
// represent information about Stratis devices. If any device with
// Stratis identifiers was in the list of devices passed to the function
// either:
// * It was filtered out of the list, because its device UUID was
// found in current_uuids
// * An error was returned because it was unsuitable, for example,
// its pool UUID did not match the pool_uuid argument
//
// Precondition: This method is called only with the result of
// process_devices. Currently, this guarantees that all LUKS devices,
// for example, have been eliminated from the devices that are being
// checked. Thus, the absence of stratisd identifiers for a particular
// device should ensure that the device does not belong to Stratis.
//
// FIXME:
// Note that this method _should_ be somewhat temporary. We hope that in
// another step the functionality contained will be hoisted up closer to
// the D-Bus/engine interface, as it computes some idempotency information.
fn check_device_ids(
    pool_uuid: PoolUuid,
    current_uuids: &HashSet<DevUuid>,
    mut devices: Vec<(InitDeviceInfo, Option<StratisIdentifiers>)>,
) -> StratisResult<Vec<InitDeviceInfo>> {
    let (mut stratis_devices, mut non_stratis_devices) = (vec![], vec![]);

    for (info, ids) in devices.drain(..) {
        match ids {
            Some(ids) => stratis_devices.push((info, ids)),
            None => non_stratis_devices.push(info),
        }
    }

    let mut pools: HashMap<PoolUuid, Vec<(DevUuid, InitDeviceInfo)>> = stratis_devices
        .drain(..)
        .fold(HashMap::new(), |mut acc, (info, identifiers)| {
            acc.entry(identifiers.pool_uuid)
                .or_insert_with(Vec::new)
                .push((identifiers.device_uuid, info));
            acc
        });

    let this_pool: Option<Vec<(DevUuid, InitDeviceInfo)>> = pools.remove(&pool_uuid);

    if !pools.is_empty() {
        let error_string = pools
            .iter()
            .map(|(pool_uuid, devs)| {
                format!(
                    "devices ({}) appear to belong to Stratis pool with UUID {}",
                    devs.iter()
                        .map(|(_, info)| info.devnode.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    pool_uuid.to_simple_ref()
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let error_message = format!(
            "Some devices specified appear to be already in use by other Stratis pools: {}",
            error_string
        );
        return Err(StratisError::Engine(ErrorEnum::Invalid, error_message));
    }

    if let Some(mut this_pool) = this_pool {
        let (mut included, mut not_included) = (vec![], vec![]);
        for (dev_uuid, info) in this_pool.drain(..) {
            if current_uuids.contains(&dev_uuid) {
                included.push((dev_uuid, info))
            } else {
                not_included.push((dev_uuid, info))
            }
        }

        if !not_included.is_empty() {
            let error_message = format!(
                "Devices ({}) appear to be already in use by this pool which has UUID {}; they may be in use by the other tier",
                not_included
                    .iter()
                    .map(|(_, info)| info.devnode.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                pool_uuid.to_simple_ref()
            );
            return Err(StratisError::Engine(ErrorEnum::Invalid, error_message));
        }

        if !included.is_empty() {
            info!(
                "Devices [{}] appear to be already in use by this pool which has UUID {}; omitting from the set of devices to initialize",
                included
                    .iter()
                    .map(|(dev_uuid, info)| {
                        format!(
                            "(device node: {}, device UUID: {})",
                            info.devnode.display().to_string(),
                            dev_uuid.to_simple_ref()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                pool_uuid.to_simple_ref()
            );
        }
    }

    Ok(non_stratis_devices)
}

/// Combine the functionality of process_devices and check_device_ids.
/// It is useful to guarantee that check_device_ids is called only with
/// the result of invoking process_devices.
pub fn process_and_verify_devices(
    pool_uuid: PoolUuid,
    current_uuids: &HashSet<DevUuid>,
    paths: &[&Path],
) -> StratisResult<Vec<InitDeviceInfo>> {
    check_device_ids(pool_uuid, current_uuids, process_devices(paths)?)
        .and_then(|vec| {
            vec
                .into_iter()
                .map(|info| {
                    if info.size < MIN_DEV_SIZE {
                        let error_message = format!(
                            "Device {} is {} which is smaller than the minimum required size for a Stratis blockdev, {}",
                            info.devnode.display(),
                            info.size,
                            MIN_DEV_SIZE);
                        Err(StratisError::Engine(ErrorEnum::Invalid, error_message))
                    } else { Ok(info) }
                })
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use crate::engine::strat_engine::tests::{loopbacked, real};

    use super::*;

    // Verify that a non-existent path results in a reasonably elegant
    // error, i.e., not an assertion failure.
    fn test_nonexistent_path(paths: &[&Path]) {
        assert!(!paths.is_empty());

        let test_paths = [paths, &[Path::new("/srk/cheese")]].concat();

        assert_matches!(process_devices(&test_paths), Err(_));
    }

    #[test]
    fn loop_test_nonexistent_path() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_nonexistent_path,
        );
    }

    #[test]
    fn real_test_nonexistent_path() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(1, None, None),
            test_nonexistent_path,
        );
    }

    #[test]
    fn travis_test_nonexistent_path() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_nonexistent_path,
        );
    }

    // Verify that resolve devices simply eliminates duplicate devnodes,
    // without returning an error.
    fn test_duplicate_devnodes(paths: &[&Path]) {
        assert!(!paths.is_empty());

        let duplicate_paths = paths
            .iter()
            .chain(paths.iter())
            .copied()
            .collect::<Vec<_>>();

        let result = process_devices(&duplicate_paths).unwrap();

        assert_eq!(result.len(), paths.len());
    }

    #[test]
    fn loop_test_duplicate_devnodes() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 2, None),
            test_duplicate_devnodes,
        );
    }

    #[test]
    fn real_test_duplicate_devnodes() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(1, None, None),
            test_duplicate_devnodes,
        );
    }

    #[test]
    fn travis_test_duplicate_devnodes() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 2, None),
            test_duplicate_devnodes,
        );
    }
}

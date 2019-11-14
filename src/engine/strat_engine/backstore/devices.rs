// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::{
    collections::HashMap,
    fs::OpenOptions,
    path::{Path, PathBuf},
};

use chrono::Utc;
use itertools::Itertools;
use uuid::Uuid;

use devicemapper::{Bytes, Device, IEC};

use crate::{
    engine::{
        engine::BlockDev,
        strat_engine::{
            backstore::{
                blockdev::StratBlockDev,
                metadata::{
                    device_identifiers, disown_device, BlockdevSize, MDADataSize,
                    StratisIdentifiers, BDA,
                },
                udev::{block_device_apply, decide_ownership, get_udev_property, UdevOwnership},
            },
            device::blkdev_size,
        },
        types::PoolUuid,
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
            .and_then(|ownership| Ok((ownership, devnum, id_wwn)))
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

            if dev_size < MIN_DEV_SIZE {
                let error_message = format!(
                    "Device {} is smaller than the minimum required size for a Stratis blockdev, {}",
                    devnode.display(),
                    MIN_DEV_SIZE);
                return Err(StratisError::Engine(ErrorEnum::Invalid, error_message));
            };

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
pub struct DeviceInfo {
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

/// Process a list of devices specified as device nodes.
///
/// * Reduce the list of devices to a set.
/// * Return a vector of accumulated information about the device nodes.
///
/// If the StratisIdentifiers value is not None, then the device has been
/// identified as a Stratis device.
///
/// Return an error if there was an error collecting the information or
/// if it turns out that at least two of the specified devices have the same
/// device number.
pub fn process_devices(
    paths: &[&Path],
) -> StratisResult<Vec<(DeviceInfo, Option<StratisIdentifiers>)>> {
    let infos = paths
        .iter()
        .unique()
        .map(|devnode| {
            dev_info(devnode).map(|(id_wwn, size, stratis_identifiers, devno)| {
                (
                    DeviceInfo {
                        devno,
                        devnode: devnode.to_path_buf(),
                        id_wwn,
                        size,
                    },
                    stratis_identifiers,
                )
            })
        })
        .collect::<StratisResult<Vec<(DeviceInfo, Option<StratisIdentifiers>)>>>()
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

/// Initialze devices in devices.
/// Clean up previously initialized devices if initialization of any single
/// device fails during initialization. Log at the warning level if cleanup
/// fails.
///
/// Precondition: All devices have been identified as ready to be initialized
/// in a previous step.
///
/// Precondition: Each device's DeviceInfo struct contains all necessary
/// information about the device.
pub fn initialize_devices(
    devices: Vec<DeviceInfo>,
    pool_uuid: PoolUuid,
    mda_data_size: MDADataSize,
    keyfile_path: Option<&Path>,
) -> StratisResult<Vec<StratBlockDev>> {
    // Initialize a single device using information in dev_info.
    // If initialization fails at any stage clean up the device.
    // Return an error if initialization failed. Log a warning if cleanup
    // fails.
    fn initialize_one(
        dev_info: &DeviceInfo,
        pool_uuid: PoolUuid,
        mda_data_size: MDADataSize,
    ) -> StratisResult<StratBlockDev> {
        let mut f = OpenOptions::new().write(true).open(&dev_info.devnode)?;

        let hw_id = match &dev_info.id_wwn {
            Some(Ok(hw_id)) => Some(hw_id.to_owned()),
            Some(Err(_)) => {
                warn!("Value for ID_WWN for device {} obtained from the udev database could not be decoded; inserting device into pool with UUID {} anyway",
                      dev_info.devnode.display(),
                      pool_uuid.to_simple_ref());
                None
            }
            None => None,
        };

        let bda = BDA::initialize(
            &mut f,
            StratisIdentifiers::new(pool_uuid, Uuid::new_v4()),
            mda_data_size,
            BlockdevSize::new(dev_info.size.sectors()),
            Utc::now().timestamp() as u64,
        );

        let blockdev = bda.and_then(|bda| {
            StratBlockDev::new(
                dev_info.devno,
                dev_info.devnode.to_owned(),
                bda,
                &[],
                None,
                hw_id,
                keyfile_path,
            )
        });

        if blockdev.is_err() {
            if let Err(err) = disown_device(&mut f) {
                warn!("Failed to clean up device {}; cleanup was attempted because initialization of the device for pool with UUID {} failed: {}",
                      dev_info.devnode.display(),
                      pool_uuid.to_simple_ref(),
                      err);
            }
        }

        blockdev
    }

    let mut initialized_blockdevs: Vec<StratBlockDev> = Vec::new();
    for dev_info in devices {
        match initialize_one(&dev_info, pool_uuid, mda_data_size) {
            Ok(blockdev) => initialized_blockdevs.push(blockdev),
            Err(err) => {
                if let Err(err) = wipe_blockdevs(&initialized_blockdevs) {
                    warn!("Failed to clean up some devices after initialization of device {} for pool with UUID {} failed: {}",
                          dev_info.devnode.display(),
                          pool_uuid.to_simple_ref(),
                          err);
                }
                return Err(err);
            }
        }
    }
    Ok(initialized_blockdevs)
}

/// Wipe some blockdevs of their identifying headers.
/// Return an error if any of the blockdevs could not be wiped.
/// If an error occurs while wiping a blockdev, attempt to wipe all remaining.
pub fn wipe_blockdevs(blockdevs: &[StratBlockDev]) -> StratisResult<()> {
    let unerased_devnodes: Vec<_> = blockdevs
        .iter()
        .filter_map(|bd| match bd.disown() {
            Err(_) => Some(bd.devnode()),
            _ => None,
        })
        .collect();

    if unerased_devnodes.is_empty() {
        Ok(())
    } else {
        let err_msg = format!(
            "Failed to wipe already initialized devnodes: {:?}",
            unerased_devnodes
        );
        Err(StratisError::Engine(ErrorEnum::Error, err_msg))
    }
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;

    use uuid::Uuid;

    use crate::engine::strat_engine::{
        backstore::{
            identify::find_all_block_devices_with_stratis_signatures, metadata::device_identifiers,
            setup::get_metadata,
        },
        cmd,
        tests::{loopbacked, real},
    };

    use super::*;

    /// Test that initialing devices claims all and that destroying
    /// them releases all.
    fn test_ownership(paths: &[&Path]) {
        let pool_uuid = Uuid::new_v4();
        let blockdevs = initialize_devices(
            process_devices(paths)
                .unwrap()
                .into_iter()
                .map(|(info, _)| info)
                .collect(),
            pool_uuid,
            MDADataSize::default(),
            None,
        )
        .unwrap();

        for path in paths {
            assert_eq!(
                pool_uuid,
                device_identifiers(&mut OpenOptions::new().read(true).open(path).unwrap(),)
                    .unwrap()
                    .unwrap()
                    .pool_uuid
            );
        }

        wipe_blockdevs(&blockdevs).unwrap();

        for path in paths {
            assert_eq!(
                device_identifiers(&mut OpenOptions::new().read(true).open(path).unwrap(),)
                    .unwrap(),
                None
            );
        }
    }

    #[test]
    fn loop_test_ownership() {
        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Range(1, 3, None), test_ownership);
    }

    #[test]
    fn real_test_ownership() {
        real::test_with_spec(&real::DeviceLimits::AtLeast(1, None, None), test_ownership);
    }

    #[test]
    fn travis_test_ownership() {
        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Range(1, 3, None), test_ownership);
    }

    /// Verify that find_all function locates and assigns pools appropriately.
    /// 1. Split available paths into 2 discrete sets.
    /// 2. Initialize the block devices in the first set with a pool uuid.
    /// 3. Run find_all() and verify that it has found the initialized devices
    /// and no others.
    /// 4. Initialize the block devices in the second set with a different pool
    /// uuid.
    /// 5. Run find_all() again and verify that both sets of devices are found.
    /// 6. Verify that get_metadata() return an error. initialize() only
    /// initializes block devices, it does not write metadata.
    // This method uses the fallback method for finding all Stratis devices,
    // since udev sometimes can not catch up to the changes made in this test
    // in the time the test allows. The fallback method has the long name
    // "find_all_block_devices_with_stratis_signatures".
    fn test_initialize(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let uuid1 = Uuid::new_v4();
        {
            let device_infos: Vec<DeviceInfo> = process_devices(paths1)
                .unwrap()
                .into_iter()
                .map(|(info, _)| info)
                .collect();

            assert_eq!(device_infos.len(), paths1.len());

            let devices =
                initialize_devices(device_infos, uuid1, MDADataSize::default(), None).unwrap();
            assert_eq!(devices.len(), paths1.len());

            for path in paths1 {
                let mut f = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(path)
                    .unwrap();
                assert_eq!(
                    device_identifiers(&mut f).unwrap().unwrap().pool_uuid,
                    uuid1
                );
            }
        }

        cmd::udev_settle().unwrap();

        {
            let pools = find_all_block_devices_with_stratis_signatures().unwrap();

            assert_eq!(pools.len(), 1);
            assert!(pools.contains_key(&uuid1));

            let devices = pools.get(&uuid1).expect("pools.contains_key() was true");

            assert_eq!(devices.len(), paths1.len());
        }

        let uuid2 = Uuid::new_v4();

        {
            let device_infos: Vec<DeviceInfo> = process_devices(paths2)
                .unwrap()
                .into_iter()
                .map(|(info, _)| info)
                .collect();

            assert_eq!(device_infos.len(), paths2.len());

            let devices =
                initialize_devices(device_infos, uuid2, MDADataSize::default(), None).unwrap();
            assert_eq!(devices.len(), paths2.len());

            for path in paths2 {
                let mut f = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(path)
                    .unwrap();
                assert_eq!(
                    device_identifiers(&mut f).unwrap().unwrap().pool_uuid,
                    uuid2
                );
            }
        }

        cmd::udev_settle().unwrap();

        {
            let pools = find_all_block_devices_with_stratis_signatures().unwrap();

            assert_eq!(pools.len(), 2);
            assert!(pools.contains_key(&uuid1));

            let devices1 = pools.get(&uuid1).expect("pools.contains_key() was true");
            assert_eq!(devices1.len(), paths1.len());
            assert!(pools.contains_key(&uuid2));

            let devices2 = pools.get(&uuid2).expect("pools.contains_key() was true");
            assert_eq!(devices2.len(), paths2.len());

            assert!(pools
                .iter()
                .map(|(uuid, devs)| get_metadata(*uuid, devs))
                .all(|x| x.unwrap().is_none()));
        }
    }

    #[test]
    fn loop_test_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_initialize,
        );
    }

    #[test]
    fn real_test_initialize() {
        real::test_with_spec(&real::DeviceLimits::AtLeast(2, None, None), test_initialize);
    }

    #[test]
    fn travis_test_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_initialize,
        );
    }

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

    // Verify that if the last device in a list of devices to intialize
    // can not be intialized, all the devices previously initialized are
    // properly cleaned up.
    fn test_failure_cleanup(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let mut devices: Vec<DeviceInfo> = process_devices(paths)
            .unwrap()
            .into_iter()
            .map(|(info, _)| info)
            .collect::<Vec<DeviceInfo>>();

        let old_info = devices.pop().unwrap();

        let new_info = DeviceInfo {
            devnode: PathBuf::from("/srk/cheese"),
            devno: old_info.devno,
            id_wwn: None,
            size: old_info.size,
        };

        devices.push(new_info);

        assert_matches!(
            initialize_devices(devices, Uuid::new_v4(), MDADataSize::default()),
            Err(_)
        );

        // Just check all paths for absence of device identifiers.
        // Initialization of the last path was never attempted, so it should
        // be as bare of Stratis identifiers as all the other paths that
        // were initialized.
        for path in paths {
            let mut f = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .unwrap();
            assert_matches!(device_identifiers(&mut f), Ok(None));
        }
    }

    #[test]
    fn loop_test_failure_cleanup() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_failure_cleanup,
        );
    }

    #[test]
    fn real_test_failure_cleanup() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_failure_cleanup,
        );
    }

    #[test]
    fn travis_test_failure_cleanup() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_failure_cleanup,
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

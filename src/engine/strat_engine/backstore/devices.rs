// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::{
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
        UdevOwnership::MultipathMember => {
            let err_str = format!(
                "udev information indicates that device {} is a multipath member device",
                devnode.display(),
            );
            Err(StratisError::Engine(ErrorEnum::Invalid, err_str))
        }
        UdevOwnership::Luks => {
            let err_str = format!(
                "udev information indicates that device {} is a LUKS encrypted device",
                devnode.display(),
            );
            Err(StratisError::Engine(ErrorEnum::Invalid, err_str))
        }
        UdevOwnership::Theirs => {
            let err_str = format!(
                "udev information indicates that device {} is not unowned",
                devnode.display(),
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
/// a device should be allowed to be initialized by Stratis.
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
    /// The device identifiers obtained from the Stratis metadata. If None,
    /// the device has been determined to be unowned.
    pub stratis_identifiers: Option<StratisIdentifiers>,
}

/// Process a list of devices specified as device nodes. Return a vector
/// of accumulated information about the device nodes.
pub fn process_devices(paths: &[&Path]) -> StratisResult<Vec<DeviceInfo>> {
    paths
        .iter()
        .map(|devnode| {
            dev_info(devnode).map(|(id_wwn, size, stratis_identifiers, devno)| DeviceInfo {
                devno,
                devnode: devnode.to_path_buf(),
                id_wwn,
                size,
                stratis_identifiers,
            })
        })
        .collect::<StratisResult<Vec<DeviceInfo>>>()
        .map_err(|err| {
            let error_message = format!(
                "At least one of the devices specified was unsuitable for initialization: {}",
                err
            );
            StratisError::Engine(ErrorEnum::Invalid, error_message)
        })
        .map(|infos| infos.into_iter().unique_by(|info| info.devno).collect())
}

pub fn initialize_devices(
    devices: Vec<DeviceInfo>,
    pool_uuid: PoolUuid,
    mda_data_size: MDADataSize,
) -> StratisResult<Vec<StratBlockDev>> {
    let mut initialized_blockdevs: Vec<StratBlockDev> = Vec::new();
    for dev_info in devices {
        let mut f = OpenOptions::new().write(true).open(&dev_info.devnode)?;
        let bda = BDA::initialize(
            &mut f,
            StratisIdentifiers::new(pool_uuid, Uuid::new_v4()),
            mda_data_size,
            BlockdevSize::new(dev_info.size.sectors()),
            Utc::now().timestamp() as u64,
        );
        if let Ok(bda) = bda {
            let hw_id = match dev_info.id_wwn {
                Some(Ok(hw_id)) => Some(hw_id),
                Some(Err(_)) => {
                    warn!("Value for ID_WWN for device {} obtained from the udev database could not be decoded; inserting device into pool with UUID {} anyway",
                          dev_info.devnode.display(),
                          pool_uuid.to_simple_ref());
                    None
                }
                None => None,
            };

            // FIXME: The expect is only provisionally true.
            // The dev_size is at least MIN_DEV_SIZE, but the size of the
            // metadata is not really bounded from above.
            let blockdev = StratBlockDev::new(
                dev_info.devno,
                dev_info.devnode.to_owned(),
                bda,
                &[],
                None,
                hw_id,
            )
            .expect("bda.size() == dev_size; only allocating space for metadata");
            initialized_blockdevs.push(blockdev);
        } else {
            // TODO: check the return values and update state machine on failure
            let _ = disown_device(&mut f);
            let _ = wipe_blockdevs(&initialized_blockdevs);

            return Err(bda.unwrap_err());
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
            process_devices(paths).unwrap(),
            pool_uuid,
            MDADataSize::default(),
        )
        .unwrap();

        cmd::udev_settle().unwrap();

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
    pub fn loop_test_ownership() {
        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Range(1, 3, None), test_ownership);
    }

    #[test]
    pub fn real_test_ownership() {
        real::test_with_spec(&real::DeviceLimits::AtLeast(1, None, None), test_ownership);
    }

    #[test]
    pub fn travis_test_ownership() {
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
            let device_infos = process_devices(paths1).unwrap();

            assert_eq!(device_infos.len(), paths1.len());

            let devices = initialize_devices(device_infos, uuid1, MDADataSize::default()).unwrap();
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
            let device_infos = process_devices(paths2).unwrap();

            assert_eq!(device_infos.len(), paths2.len());

            let devices = initialize_devices(device_infos, uuid2, MDADataSize::default()).unwrap();
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
    pub fn loop_test_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_initialize,
        );
    }

    #[test]
    pub fn real_test_initialize() {
        real::test_with_spec(&real::DeviceLimits::AtLeast(2, None, None), test_initialize);
    }

    #[test]
    pub fn travis_test_initialize() {
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
    pub fn loop_test_nonexistent_path() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_nonexistent_path,
        );
    }

    #[test]
    pub fn real_test_nonexistent_path() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(1, None, None),
            test_nonexistent_path,
        );
    }

    #[test]
    pub fn travis_test_nonexistent_path() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_nonexistent_path,
        );
    }
}

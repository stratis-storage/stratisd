// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::{
    collections::{HashMap, HashSet},
    fs::{File, OpenOptions},
    path::Path,
};

use chrono::Utc;
use uuid::Uuid;

use devicemapper::{devnode_to_devno, Bytes, Device, IEC};

use crate::{
    engine::{
        engine::BlockDev,
        strat_engine::{
            backstore::{
                blockdev::StratBlockDev,
                device::DevOwnership,
                metadata::{disown_device, BlockdevSize, MDADataSize, BDA},
                udev::{block_device_apply, decide_ownership, get_udev_property},
            },
            device::blkdev_size,
        },
        types::{DevUuid, PoolUuid},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

const MIN_DEV_SIZE: Bytes = Bytes(IEC::Gi);

/// Resolve a list of Paths of some sort to a set of unique Devices.
/// Return an IOError if there was a problem resolving any particular device.
/// The set of devices maps each device to one of the paths passed.
/// Returns an error if any path does not correspond to a block device.
pub fn resolve_devices<'a>(paths: &'a [&Path]) -> StratisResult<HashMap<Device, &'a Path>> {
    let mut map = HashMap::new();
    for path in paths {
        match devnode_to_devno(path)? {
            Some(devno) => {
                let _ = map.insert(Device::from(devno), *path);
            }
            None => {
                let err_msg = format!("path {} does not refer to a block device", path.display());
                return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
            }
        }
    }
    Ok(map)
}

/// Initialize multiple blockdevs at once. This allows all of them
/// to be checked for usability before writing to any of them.
pub fn initialize(
    pool_uuid: PoolUuid,
    devices: HashMap<Device, &Path>,
    mda_data_size: MDADataSize,
    owned_devs: &HashSet<DevUuid>,
) -> StratisResult<Vec<StratBlockDev>> {
    /// Get device information, returns an error if problem with obtaining
    /// that information.
    /// Returns a tuple with the device's path, its size in bytes,
    /// its DevOwnership classification, its optional hw_id,
    /// and an open File handle.
    #[allow(clippy::type_complexity)]
    fn dev_info(
        devnode: &Path,
    ) -> StratisResult<(
        &Path,
        Bytes,
        StratisResult<DevOwnership>,
        Option<StratisResult<String>>,
        File,
    )> {
        let f = OpenOptions::new().read(true).write(true).open(&devnode)?;
        let dev_size = blkdev_size(&f)?;

        if let Some((ownership, hw_id)) = block_device_apply(devnode, |d| {
            (
                decide_ownership(d)
                    .and_then(|decision| DevOwnership::from_udev_ownership(&decision, devnode)),
                get_udev_property(d, "ID_WWN"),
            )
        })? {
            Ok((devnode, dev_size, ownership, hw_id, f))
        } else {
            Err(StratisError::Engine(
                ErrorEnum::NotFound,
                format!(
                    "Could not determine ownership of block device {} because it could not be found in the udev database",
                    devnode.display()
                ),
            ))
        }
    }

    /// Filter devices for admission to pool based on dev_infos.
    /// If there is an error finding out the info, return that error.
    /// Also, return an error if a device is not appropriate for this pool.
    #[allow(clippy::type_complexity)]
    fn filter_devs<'a, I>(
        dev_infos: I,
        pool_uuid: PoolUuid,
        owned_devs: &HashSet<DevUuid>,
    ) -> StratisResult<
        Vec<(
            Device,
            (&'a Path, Bytes, Option<StratisResult<String>>, File),
        )>,
    >
    where
        I: Iterator<
            Item = (
                Device,
                StratisResult<(
                    &'a Path,
                    Bytes,
                    StratisResult<DevOwnership>,
                    Option<StratisResult<String>>,
                    File,
                )>,
            ),
        >,
    {
        let mut add_devs = Vec::new();
        for (dev, dev_result) in dev_infos {
            let (devnode, dev_size, ownership, hw_id, f) = dev_result?;
            if dev_size < MIN_DEV_SIZE {
                let error_message =
                    format!("{} too small, minimum {}", devnode.display(), MIN_DEV_SIZE);
                return Err(StratisError::Engine(ErrorEnum::Invalid, error_message));
            };
            match ownership {
                Ok(DevOwnership::Unowned) => add_devs.push((dev, (devnode, dev_size, hw_id, f))),
                Ok(DevOwnership::Theirs(info)) => {
                    let err_str = format!(
                        "Device {} appears to be already claimed by another, reason: {}",
                        devnode.display(),
                        info
                    );
                    return Err(StratisError::Engine(ErrorEnum::Invalid, err_str));
                }
                Ok(DevOwnership::Ours(uuid, dev_uuid)) => {
                    if pool_uuid == uuid {
                        if !owned_devs.contains(&dev_uuid) {
                            let error_str = format!(
                                "Device {} with pool UUID is unknown to pool",
                                devnode.display()
                            );
                            return Err(StratisError::Engine(ErrorEnum::Invalid, error_str));
                        }
                    } else {
                        let error_str = format!(
                            "Device {} already belongs to Stratis pool {}",
                            devnode.display(),
                            uuid
                        );
                        return Err(StratisError::Engine(ErrorEnum::Invalid, error_str));
                    }
                }
                Err(err) => {
                    let error_str = format!(
                        "Unable to obtain ownership information for device {}: {}",
                        devnode.display(),
                        err
                    );
                    return Err(StratisError::Error(error_str));
                }
            }
        }
        Ok(add_devs)
    }

    let dev_infos = devices.into_iter().map(|(d, p)| (d, dev_info(p)));

    let add_devs = filter_devs(dev_infos, pool_uuid, owned_devs)?;

    let mut bds: Vec<StratBlockDev> = Vec::new();
    for (dev, (devnode, dev_size, hw_id, mut f)) in add_devs {
        let bda = BDA::initialize(
            &mut f,
            pool_uuid,
            Uuid::new_v4(),
            mda_data_size,
            BlockdevSize::new(dev_size.sectors()),
            Utc::now().timestamp() as u64,
        );
        if let Ok(bda) = bda {
            let hw_id = match hw_id {
                Some(Ok(hw_id)) => Some(hw_id),
                Some(Err(_)) => {
                    warn!("Value for ID_WWN for device {} obtained from the udev database could not be decoded; inserting device into pool with UUID {} anyway",
                          devnode.display(),
                          pool_uuid.to_simple_ref());
                    None
                }
                None => None,
            };

            // FIXME: The expect is only provisionally true.
            // The dev_size is at least MIN_DEV_SIZE, but the size of the
            // metadata is not really bounded from above.
            let blockdev = StratBlockDev::new(dev, devnode.to_owned(), bda, &[], None, hw_id)
                .expect("bda.size() == dev_size; only allocating space for metadata");
            bds.push(blockdev);
        } else {
            // TODO: check the return values and update state machine on failure
            let _ = disown_device(&mut f);
            let _ = wipe_blockdevs(&bds);

            return Err(bda.unwrap_err());
        }
    }
    Ok(bds)
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

    use rand;
    use uuid::Uuid;

    use crate::engine::strat_engine::{
        backstore::metadata::device_identifiers,
        cmd,
        tests::{loopbacked, real},
    };

    use super::*;

    /// Verify that it is impossible to initialize a set of disks of which
    /// even one of them has a signature.  Choose the dirty disk randomly.
    fn test_fail_single_signature(paths: &[&Path]) {
        assert!(paths.len() > 1);
        let index = rand::random::<u8>() as usize % paths.len();

        cmd::create_fs(paths[index], None).unwrap();
        cmd::udev_settle().unwrap();

        let pool_uuid = Uuid::new_v4();
        assert_matches!(
            initialize(
                pool_uuid,
                resolve_devices(paths).unwrap(),
                MDADataSize::default(),
                &HashSet::new()
            ),
            Err(_)
        );
        for (i, path) in paths.iter().enumerate() {
            if i == index {
                assert_matches!(
                    DevOwnership::from_udev_ownership(
                        &block_device_apply(path, |d| decide_ownership(d))
                            .unwrap()
                            .unwrap()
                            .unwrap(),
                        path
                    )
                    .unwrap(),
                    DevOwnership::Theirs(_)
                );
            } else {
                assert_matches!(
                    DevOwnership::from_udev_ownership(
                        &block_device_apply(path, |d| decide_ownership(d))
                            .unwrap()
                            .unwrap()
                            .unwrap(),
                        path
                    )
                    .unwrap(),
                    DevOwnership::Unowned
                );
            }
        }

        let clean_paths = paths
            .iter()
            .enumerate()
            .filter(|(n, _)| *n != index)
            .map(|(_, v)| *v)
            .collect::<Vec<&Path>>();

        assert_matches!(
            initialize(
                pool_uuid,
                resolve_devices(&clean_paths).unwrap(),
                MDADataSize::default(),
                &HashSet::new()
            ),
            Ok(_)
        );
        cmd::udev_settle().unwrap();

        for path in clean_paths {
            assert_eq!(
                pool_uuid,
                device_identifiers(&mut OpenOptions::new().read(true).open(path).unwrap(),)
                    .unwrap()
                    .unwrap()
                    .0
            );
        }
    }

    #[test]
    pub fn loop_test_fail_single_signature() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_fail_single_signature,
        );
    }

    #[test]
    pub fn real_test_fail_single_signature() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_fail_single_signature,
        );
    }

    #[test]
    pub fn travis_test_fail_single_signature() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_fail_single_signature,
        );
    }

    /// Test that initialing devices claims all and that destroying
    /// them releases all.
    fn test_ownership(paths: &[&Path]) {
        let pool_uuid = Uuid::new_v4();
        let blockdevs = initialize(
            pool_uuid,
            resolve_devices(paths).unwrap(),
            MDADataSize::default(),
            &HashSet::new(),
        )
        .unwrap();

        cmd::udev_settle().unwrap();

        for path in paths {
            assert_eq!(
                pool_uuid,
                device_identifiers(&mut OpenOptions::new().read(true).open(path).unwrap(),)
                    .unwrap()
                    .unwrap()
                    .0
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
}

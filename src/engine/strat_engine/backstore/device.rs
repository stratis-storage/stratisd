// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::os::unix::prelude::AsRawFd;
use std::path::Path;

use libudev;

use devicemapper::{devnode_to_devno, Bytes, Device};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::types::{DevUuid, PoolUuid};

use super::metadata::device_identifiers;
use super::udev::{udev_block_device_apply, unclaimed};

ioctl_read!(blkgetsize64, 0x12, 114, u64);

pub fn blkdev_size(file: &File) -> StratisResult<Bytes> {
    let mut val: u64 = 0;

    match unsafe { blkgetsize64(file.as_raw_fd(), &mut val) } {
        Err(x) => Err(StratisError::Nix(x)),
        Ok(_) => Ok(Bytes(val)),
    }
}

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

#[derive(Debug, PartialEq, Eq)]
/// Designations of device ownership
pub enum DevOwnership {
    /// Udev thinks the device belongs to Stratis, but Stratis does not
    Contradiction,
    /// Udev believes that the device is owned by multipath
    Multipath,
    /// Udev and Stratis agree that the device belongs to Stratis, these
    /// are the device's pool and device UUID.
    Ours(PoolUuid, DevUuid),
    /// Udev believes that the device is unowned.
    Unowned,
    /// Udev believes that the device is owned by something other than
    /// Stratis, and the constructor argument contains some relevant
    /// udev properties.
    Theirs(HashMap<String, String>),
}

/// Identify a device node using a combination of udev information and
/// Stratis signature information.
/// Return an error if the device is not in the udev database.
/// Return an error if the necessary udev information can not be read.
pub fn identify(devnode: &Path) -> StratisResult<DevOwnership> {
    /// A helper function. None if the device is unclaimed, a HashMap of udev
    /// properties otherwise. Omits all udev properties that can not be
    /// converted to Strings.
    fn udev_info(device: &libudev::Device) -> Option<HashMap<String, Option<String>>> {
        if unclaimed(device) {
            None
        } else {
            Some(
                device
                    .properties()
                    .map(|i| {
                        (
                            i.name().to_str().map(|s| s.to_string()),
                            i.value().to_str().map(|s| s.to_string()),
                        )
                    })
                    .filter_map(|(n, v)| match (n, v) {
                        (Some(n), v) => Some((n, v)),
                        _ => None,
                    })
                    .collect(),
            )
        }
    }

    match udev_block_device_apply(devnode, udev_info)? {
        Some(Some(properties)) => {
            if properties
                .get("DM_MULTIPATH_DEVICE_PATH")
                .map_or(false, |v| v.as_ref().map_or(false, |v| v == "1"))
            {
                Ok(DevOwnership::Multipath)
            } else if properties
                .get("ID_FS_TYPE")
                .map_or(false, |v| v.as_ref().map_or(false, |v| v == "stratis"))
            {
                if let Some((pool_uuid, device_uuid)) =
                    device_identifiers(&mut OpenOptions::new().read(true).open(&devnode)?)?
                {
                    Ok(DevOwnership::Ours(pool_uuid, device_uuid))
                } else {
                    Ok(DevOwnership::Contradiction)
                }
            } else {
                Ok(DevOwnership::Theirs(
                    properties
                        .iter()
                        .filter(|&(k, _)| k.contains("ID_FS_") || k.contains("ID_PART_TABLE_"))
                        .map(|(k, v)| {
                            (
                                k.to_string(),
                                v.as_ref()
                                    .map_or("<unknown value>".into(), |v| v.to_string()),
                            )
                        })
                        .collect(),
                ))
            }
        }
        Some(None) => {
            // Not a Stratis device OR running an older version of libblkid
            // that does not interpret Stratis devices. Fall back on reading
            // Stratis header via Stratis. Be more accepting of failures
            // while reading the Stratis header from the device than in
            // the case where udev has actually identified this device as
            // belonging to Stratis.
            //
            // NOTE: This is a bit kludgy. If, at any time during stratisd
            // execution, a device is identified as a Stratis device by libblkid
            // then it is clear that the version of libblkid being run is new
            // enough. But to track that information requires some kind of
            // stateful global variable. So, instead, fall back on the safe
            // approach of just always reading the Stratis header, regardless
            // of what has happened in the past.
            Ok(
                if let Ok(Some((pool_uuid, device_uuid))) = OpenOptions::new()
                    .read(true)
                    .open(&devnode)
                    .map_err(|e| e.into())
                    .and_then(|mut file| device_identifiers(&mut file))
                {
                    DevOwnership::Ours(pool_uuid, device_uuid)
                } else {
                    DevOwnership::Unowned
                },
            )
        }
        None => Err(StratisError::Engine(
            ErrorEnum::NotFound,
            format!(
                "No device in udev database corresponding to device node {:?}",
                devnode
            ),
        )),
    }
}

#[cfg(test)]
mod test {

    use std::path::Path;

    use super::super::super::cmd;
    use super::super::super::tests::{loopbacked, real};

    use super::*;

    /// Verify that a device with an ext3 filesystem directly on it is
    /// identified as not a Stratis device.
    fn test_other_ownership(paths: &[&Path]) {
        cmd::create_ext3_fs(paths[0]).unwrap();

        cmd::udev_settle().unwrap();

        assert!(match identify(paths[0]).unwrap() {
            DevOwnership::Theirs(properties) => {
                assert_eq!(
                    properties.get("ID_FS_USAGE"),
                    Some(&"filesystem".to_string())
                );
                assert_eq!(properties.get("ID_FS_TYPE"), Some(&"ext3".to_string()));
                assert!(properties.get("ID_FS_UUID").is_some());
                true
            }
            _ => false,
        })
    }

    #[test]
    pub fn loop_test_other_ownership() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(1, 3, None),
            test_other_ownership,
        );
    }

    #[test]
    pub fn travis_test_other_ownership() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(1, 3, None),
            test_other_ownership,
        );
    }

    #[test]
    pub fn real_test_other_ownership() {
        real::test_with_spec(
            real::DeviceLimits::AtLeast(1, None, None),
            test_other_ownership,
        );
    }

    /// Verify that an empty device is unowned.
    fn test_empty(paths: &[&Path]) {
        cmd::udev_settle().unwrap();

        assert!(match identify(paths[0]).unwrap() {
            DevOwnership::Unowned => true,
            _ => false,
        });
    }

    #[test]
    pub fn loop_test_device_empty() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(1, 3, None), test_empty);
    }

    #[test]
    pub fn travis_test_device_empty() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(1, 3, None), test_empty);
    }

    #[test]
    pub fn real_test_device_empty() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1, None, None), test_empty);
    }
}

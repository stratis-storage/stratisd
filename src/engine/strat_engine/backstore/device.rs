// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::os::unix::prelude::AsRawFd;
use std::path::Path;

use devicemapper::{devnode_to_devno, Bytes, Device};
use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::types::{DevUuid, PoolUuid};
use super::metadata::StaticHeader;
use super::util::get_udev_block_device;

ioctl!(read blkgetsize64 with 0x12, 114; u64);

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
pub enum DevOwnership {
    Ours(PoolUuid, DevUuid),
    Unowned,
    Theirs(String), // String is something useful to give back to end user about what's on device
}

/// Returns true if a device has no signature, yes this is a bit convoluted.  Logic gleaned from
/// blivet library.
fn empty(device: &HashMap<String, String>) -> bool {
    !((device.contains_key("ID_PART_TABLE_TYPE") && !device.contains_key("ID_PART_ENTRY_DISK"))
        || device.contains_key("ID_FS_USAGE"))
}

/// Generate some kind of human readable text about what's on a device.
fn signature(device: &HashMap<String, String>) -> String {
    if empty(device) {
        String::from("empty")
    } else {
        device
            .iter()
            .filter(|&(k, _)| k.contains("ID_FS_") || k.contains("ID_PART_TABLE_"))
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<String>>()
            .join(" ")
    }
}

/// Determine what a block device is used for.
pub fn identify(devnode: &Path) -> StratisResult<DevOwnership> {
    if let Some(device) = get_udev_block_device(devnode)? {
        if empty(&device) {
            // The device is either really empty or we are running on a distribution that hasn't
            // picked up the latest libblkid, lets read down to the device and find out for sure.
            // TODO: At some point in the future we can remove this and just return Unowned.
            if let Some((pool_uuid, device_uuid)) = StaticHeader::device_identifiers(
                &mut OpenOptions::new().read(true).open(&devnode)?,
            )? {
                Ok(DevOwnership::Ours(pool_uuid, device_uuid))
            } else {
                Ok(DevOwnership::Unowned)
            }
        } else if device.contains_key("ID_FS_TYPE")
            && device.get("ID_FS_TYPE").unwrap() == "stratis"
        {
            // Device is ours, but we don't get everything we need from udev db, lets go to disk.
            if let Some((pool_uuid, device_uuid)) = StaticHeader::device_identifiers(
                &mut OpenOptions::new().read(true).open(&devnode)?,
            )? {
                Ok(DevOwnership::Ours(pool_uuid, device_uuid))
            } else {
                // In this case the udev db says it's ours, but our check says otherwise.  We should
                // trust ourselves.  Should we raise an error here?
                Ok(DevOwnership::Theirs(String::from(
                    "Udev db says stratis, disk meta says no",
                )))
            }
        } else {
            Ok(DevOwnership::Theirs(signature(&device)))
        }
    } else {
        Err(StratisError::Engine(
            ErrorEnum::NotFound,
            String::from(format!(
                "We expected to find the block device {:?} in the udev db",
                devnode
            )),
        ))
    }
}

/// Determine if devnode is a Stratis device. Return the device's Stratis
/// pool UUID if it belongs to Stratis.
pub fn is_stratis_device(devnode: &Path) -> StratisResult<Option<PoolUuid>> {
    match identify(devnode)? {
        DevOwnership::Ours(pool_uuid, _) => Ok(Some(pool_uuid)),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::super::super::cmd::{udev_settle, create_ext3_fs};
    use super::super::super::tests::{loopbacked, real};

    use super::super::device;

    /// Verify that the device is not stratis by creating a device with XFS fs.
    fn test_other_ownership(paths: &[&Path]) {
        create_ext3_fs(paths[0]).unwrap();

        udev_settle().unwrap();

        assert_eq!(device::is_stratis_device(paths[0]).unwrap(), None);

        assert!(match device::identify(paths[0]).unwrap() {
            device::DevOwnership::Theirs(identity) => {
                assert!(identity.contains("ID_FS_USAGE=filesystem"));
                assert!(identity.contains("ID_FS_TYPE=ext3"));
                assert!(identity.contains("ID_FS_UUID"));
                true
            }
            _ => false,
        });
    }

    /// Test a blank device and ensure it comes up as device::Usage::Unowned
    fn test_empty(paths: &[&Path]) {
        udev_settle().unwrap();

        assert_eq!(device::is_stratis_device(paths[0]).unwrap(), None);

        assert!(match device::identify(paths[0]).unwrap() {
            device::DevOwnership::Unowned => true,
            _ => false,
        });

        assert_eq!(device::is_stratis_device(paths[0]).unwrap(), None);
    }

    #[test]
    pub fn loop_test_device_other_ownership() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(1, 3, None),
            test_other_ownership,
        );
    }

    #[test]
    pub fn real_test_device_other_ownership() {
        real::test_with_spec(
            real::DeviceLimits::AtLeast(1, None, None),
            test_other_ownership,
        );
    }

    #[test]
    pub fn loop_test_device_empty() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(1, 3, None), test_empty);
    }

    #[test]
    pub fn real_test_device_empty() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1, None, None), test_empty);
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::{collections::HashMap, fs::OpenOptions, path::Path};

use devicemapper::{devnode_to_devno, Device};

use crate::{
    engine::{
        strat_engine::backstore::{metadata::device_identifiers, udev::get_udev_block_device},
        types::{DevUuid, PoolUuid},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

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

impl DevOwnership {
    pub fn stratis_identifiers(&self) -> Option<(PoolUuid, DevUuid)> {
        match self {
            DevOwnership::Ours(pool_uuid, dev_uuid) => Some((*pool_uuid, *dev_uuid)),
            _ => None,
        }
    }
}

/// Returns true if a device has no signature and is not one of the paths of a multipath device,
/// yes this is a bit convoluted.  Logic gleaned from blivet library.
fn empty(device: &HashMap<String, String>) -> bool {
    device
        .get("DM_MULTIPATH_DEVICE_PATH")
        .map_or(true, |v| v != "1")
        && !((device.contains_key("ID_PART_TABLE_TYPE")
            && !device.contains_key("ID_PART_ENTRY_DISK"))
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
            if let Some((pool_uuid, device_uuid)) =
                device_identifiers(&mut OpenOptions::new().read(true).open(&devnode)?)?
            {
                Ok(DevOwnership::Ours(pool_uuid, device_uuid))
            } else {
                Ok(DevOwnership::Unowned)
            }
        } else if device
            .get("DM_MULTIPATH_DEVICE_PATH")
            .map_or(false, |v| v == "1")
        {
            Ok(DevOwnership::Theirs(String::from("multipath path")))
        } else if device.contains_key("ID_FS_TYPE") && device["ID_FS_TYPE"] == "stratis" {
            // Device is ours, but we don't get everything we need from udev db, lets go to disk.
            if let Some((pool_uuid, device_uuid)) =
                device_identifiers(&mut OpenOptions::new().read(true).open(&devnode)?)?
            {
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
            format!(
                "We expected to find the block device {:?} \
                 in the udev db",
                devnode
            ),
        ))
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use crate::engine::strat_engine::{
        cmd,
        tests::{loopbacked, real},
    };

    use super::*;

    /// Verify that the device is not stratis by creating a device with XFS fs.
    fn test_other_ownership(paths: &[&Path]) {
        cmd::create_fs(paths[0], None).unwrap();

        cmd::udev_settle().unwrap();

        match identify(paths[0]).unwrap() {
            DevOwnership::Theirs(identity) => {
                assert!(identity.contains("ID_FS_USAGE=filesystem"));
                assert!(identity.contains("ID_FS_TYPE=xfs"));
                assert!(identity.contains("ID_FS_UUID"));
            }
            // This must fail, and will give a helpful error message
            id => assert_matches!(id, DevOwnership::Theirs(_)),
        }
    }

    /// Verify that identify() recognizes a blank device as unowned
    fn test_empty(paths: &[&Path]) {
        cmd::udev_settle().unwrap();
        assert_matches!(identify(paths[0]).unwrap(), DevOwnership::Unowned);
    }

    #[test]
    pub fn loop_test_device_other_ownership() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_other_ownership,
        );
    }

    #[test]
    pub fn real_test_device_other_ownership() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(1, None, None),
            test_other_ownership,
        );
    }

    #[test]
    pub fn loop_test_device_empty() {
        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Range(1, 3, None), test_empty);
    }

    #[test]
    pub fn real_test_device_empty() {
        real::test_with_spec(&real::DeviceLimits::AtLeast(1, None, None), test_empty);
    }
}

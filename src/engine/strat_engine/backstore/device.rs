// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::{collections::HashMap, fs::OpenOptions, path::Path};

use devicemapper::{devnode_to_devno, Device};

use crate::{
    engine::{
        strat_engine::{backstore::metadata::device_identifiers, udev::UdevOwnership},
        types::{DevUuid, PoolUuid},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

#[cfg(test)]
use crate::engine::strat_engine::udev::{block_enumerator, decide_ownership};

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
    // FIXME: Using a string for identification of specific subclasses of
    // ownership is a mistake, an enum would be better.
    Theirs(String), // A string that is supposed to give more description
}

impl DevOwnership {
    pub fn stratis_identifiers(&self) -> Option<(PoolUuid, DevUuid)> {
        match self {
            DevOwnership::Ours(pool_uuid, dev_uuid) => Some((*pool_uuid, *dev_uuid)),
            _ => None,
        }
    }

    /// Given a udev assignment of ownership and the devnode for the device
    /// in question, do some additional work to determine DevOwnership.
    pub fn from_udev_ownership(
        ownership: &UdevOwnership,
        devnode: &Path,
    ) -> StratisResult<DevOwnership> {
        match ownership {
            UdevOwnership::Unowned => {
                // FIXME: It is possible that Stratis is running in
                // an old environment without the necessary version of
                // libblkid that would cause udev database to be
                // populated with Stratis information. So, if the
                // device appears unowned, attempt to read information
                // from Stratis metadata. We believe that this block
                // can be removed once Stratis is certainly runnng
                // with libblkid 2.32 or above.
                // https://github.com/stratis-storage/stratisd/issues/1656
                if let Some((pool_uuid, device_uuid)) =
                    device_identifiers(&mut OpenOptions::new().read(true).open(&devnode)?)?
                {
                    Ok(DevOwnership::Ours(pool_uuid, device_uuid))
                } else {
                    Ok(DevOwnership::Unowned)
                }
            }
            UdevOwnership::MultipathMember => Ok(DevOwnership::Theirs("multipath member".into())),
            UdevOwnership::Stratis => {
                // Udev information does not include pool UUID and
                // device UUID so read these from Stratis metadata.
                if let Some((pool_uuid, device_uuid)) =
                    device_identifiers(&mut OpenOptions::new().read(true).open(&devnode)?)?
                {
                    Ok(DevOwnership::Ours(pool_uuid, device_uuid))
                } else {
                    // FIXME: if udev says stratis but no stratis
                    // idenfiers on device, likely they were there
                    // recently, and udev has not yet caught up. It's
                    // just as likely that this device is unclaimed as
                    // that it belongs to some other entity.
                    Ok(DevOwnership::Theirs(
                        "Udev db says stratis, disk meta says no".into(),
                    ))
                }
            }
            UdevOwnership::Theirs => Ok(DevOwnership::Theirs(
                "udev properties for this device did not indicate that the device was unowned"
                    .into(),
            )),
        }
    }
}

/// Determine what a block device is used for.
#[cfg(test)]
pub fn identify(devnode: &Path) -> StratisResult<DevOwnership> {
    let canonical = devnode.canonicalize()?;

    let context = libudev::Context::new()?;
    let mut enumerator = block_enumerator(&context)?;
    if let Some(udev_decision) = enumerator
        .scan_devices()?
        .filter(|dev| dev.is_initialized())
        .find(|x| x.devnode().map_or(false, |d| canonical == d))
        .map(|d| decide_ownership(&d))
    {
        udev_decision.and_then(|decision| DevOwnership::from_udev_ownership(&decision, &canonical))
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
        assert_matches!(identify(paths[0]).unwrap(), DevOwnership::Theirs(_))
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

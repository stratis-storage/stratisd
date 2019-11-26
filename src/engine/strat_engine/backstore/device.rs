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

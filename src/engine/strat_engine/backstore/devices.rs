// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::{collections::HashMap, path::Path};

use devicemapper::{devnode_to_devno, Device};

use crate::stratis::{ErrorEnum, StratisError, StratisResult};

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

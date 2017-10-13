// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Utilities to support Stratis.

use std::path::Path;
use std::process::Command;

use uuid::Uuid;

use super::super::errors::{EngineError, EngineResult, ErrorEnum};


/// Create a filesystem on devnode.
pub fn create_fs(devnode: &Path, uuid: Uuid) -> EngineResult<()> {
    if Command::new("mkfs.xfs")
           .arg("-f")
           .arg("-q")
           .arg(&devnode)
           .arg("-m")
           .arg(format!("uuid={}", uuid))
           .status()?
           .success() {
        Ok(())
    } else {
        let err_msg = format!("Failed to create new filesystem at {:?}", devnode);
        Err(EngineError::Engine(ErrorEnum::Error, err_msg))
    }
}

/// Use the xfs_growfs command to expand a filesystem mounted at the given
/// mount point.
pub fn xfs_growfs(mount_point: &Path) -> EngineResult<()> {
    if Command::new("xfs_growfs")
           .arg(mount_point)
           .arg("-d")
           .status()?
           .success() {
        Ok(())
    } else {
        let err_msg = format!("Failed to expand filesystem {:?}", mount_point);
        Err(EngineError::Engine(ErrorEnum::Error, err_msg))
    }
}

/// Set a new UUID for filesystem on the devnode.
pub fn set_uuid(devnode: &Path, uuid: Uuid) -> EngineResult<()> {
    if Command::new("xfs_admin")
           .arg("-U")
           .arg(format!("{}", uuid))
           .arg(&devnode)
           .status()?
           .success() {
        Ok(())
    } else {
        let err_msg = format!("Failed to set UUID for filesystem at {:?}", devnode);
        Err(EngineError::Engine(ErrorEnum::Error, err_msg))
    }
}

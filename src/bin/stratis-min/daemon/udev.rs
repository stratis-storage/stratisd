// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use regex::Regex;

use libstratis::{
    engine::{Engine, FilesystemUuid, PoolUuid, StratEngine},
    stratis::{StratisError, StratisResult},
};

pub fn udev_with_err(dm_name: &str) -> StratisResult<()> {
    let regex = Regex::new("stratis-1-([0-9a-f]{32})-thin-fs-([0-9a-f]{32})")
        .map_err(|e| StratisError::Error(e.to_string()))?;
    if let Some(captures) = regex.captures(dm_name) {
        let pool_uuid = &captures[1];
        let fs_uuid = &captures[2];
        let engine = StratEngine::initialize()?;
        let (pool_name, pool) = engine
            .get_pool(PoolUuid::parse_str(pool_uuid)?)
            .ok_or_else(|| {
                StratisError::Error(format!("Pool with UUID {} not found", pool_uuid))
            })?;
        let (fs_name, _) = pool
            .get_filesystem(FilesystemUuid::parse_str(fs_uuid)?)
            .ok_or_else(|| {
                StratisError::Error(format!("Filesystem with UUID {} not found", fs_uuid))
            })?;
        println!("STRATIS_SYMLINK=stratis/{}/{}", pool_name, fs_name);
    }
    Ok(())
}

pub fn udev(dm_name: &str) -> Result<(), String> {
    udev_with_err(dm_name).map_err(|e| e.to_string())?;
    Ok(())
}

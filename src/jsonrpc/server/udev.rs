// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use regex::Regex;

use crate::{
    engine::{Engine, FilesystemUuid, PoolUuid, StratEngine},
    stratis::{StratisError, StratisResult},
};

pub fn udev(engine: &mut StratEngine, dm_name: &str) -> StratisResult<Option<(String, String)>> {
    let regex = Regex::new("stratis-1-([0-9a-f]{32})-thin-fs-([0-9a-f]{32})")
        .map_err(|e| StratisError::Error(e.to_string()))?;
    if let Some(captures) = regex.captures(dm_name) {
        let pool_uuid = &captures[1];
        let fs_uuid = &captures[2];
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
        Ok(Some((pool_name.to_string(), fs_name.to_string())))
    } else {
        Ok(None)
    }
}

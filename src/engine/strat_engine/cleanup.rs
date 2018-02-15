// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle cleanup after a failed operation.

use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::structures::Table;

use super::blockdev::StratBlockDev;
use super::pool::StratPool;

/// Wipe some blockdevs of their identifying headers.
/// Return an error if any of the blockdevs could not be wiped.
/// If an error occurs while wiping a blockdev, attempt to wipe all remaining.
pub fn wipe_blockdevs(blockdevs: &[StratBlockDev]) -> EngineResult<()> {
    let mut unerased_devnodes = Vec::new();

    for bd in blockdevs {
        let bd_devnode = bd.devnode.to_owned();
        bd.wipe_metadata()
            .unwrap_or_else(|_| unerased_devnodes.push(bd_devnode));
    }

    if unerased_devnodes.is_empty() {
        Ok(())
    } else {
        let err_msg = format!("Failed to wipe already initialized devnodes: {:?}",
                              unerased_devnodes);
        Err(EngineError::Engine(ErrorEnum::Error, err_msg))
    }
}

/// Teardown pools.
pub fn teardown_pools(pools: Table<StratPool>) -> EngineResult<()> {
    let mut untorndown_pools = Vec::new();
    for (_, uuid, pool) in pools {
        pool.teardown()
            .unwrap_or_else(|_| untorndown_pools.push(uuid));
    }
    if untorndown_pools.is_empty() {
        Ok(())
    } else {
        let err_msg = format!("Failed to teardown already set up pools: {:?}",
                              untorndown_pools);
        Err(EngineError::Engine(ErrorEnum::Error, err_msg))
    }
}

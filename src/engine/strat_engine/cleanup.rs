// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle cleanup after a failed operation.


use super::super::engine::HasUuid;
use super::super::errors::{EngineResult, EngineError, ErrorEnum};

use super::blockdev::BlockDev;
use super::pool::StratPool;

/// Wipe a Vec of blockdevs of their identifying headers.
/// Return an error if any of the blockdevs could not be wiped.
/// If an error occurs while wiping a blockdev, attempt to wipe all remaining.
pub fn wipe_blockdevs(mut blockdevs: Vec<BlockDev>) -> EngineResult<()> {
    let mut unerased_devnodes = Vec::new();

    for bd in blockdevs.drain(..) {
        let bd_devnode = bd.devnode.clone();
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
pub fn teardown_pools(mut pools: Vec<StratPool>) -> EngineResult<()> {
    let mut untorndown_pools = Vec::new();
    for pool in pools.drain(..) {
        let pool_uuid = *pool.uuid();
        pool.teardown()
            .unwrap_or_else(|_| untorndown_pools.push(pool_uuid));
    }
    if untorndown_pools.is_empty() {
        Ok(())
    } else {
        let err_msg = format!("Failed to teardown already set up pools: {:?}",
                              untorndown_pools);
        Err(EngineError::Engine(ErrorEnum::Error, err_msg))
    }
}

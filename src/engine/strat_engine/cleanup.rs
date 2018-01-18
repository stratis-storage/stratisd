// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle cleanup after a failed operation.

use super::super::engine::HasUuid;
use super::super::errors::{EngineResult, EngineError, ErrorEnum};

use super::pool::StratPool;


/// Teardown pools.
pub fn teardown_pools(pools: Vec<StratPool>) -> EngineResult<()> {
    let mut untorndown_pools = Vec::new();
    for pool in pools {
        let pool_uuid = pool.uuid();
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

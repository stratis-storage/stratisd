// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle cleanup after a failed operation.

use super::super::errors::{StratisError, EngineResult, ErrorEnum};
use super::super::structures::Table;

use super::pool::StratPool;


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
        Err(StratisError::Engine(ErrorEnum::Error, err_msg))
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Simple tests, generally showing that some impossible situation causes
/// an error to be returned.


use std::path::Path;

use libstratis::engine::{Engine, EngineError, ErrorEnum};
use libstratis::engine::strat_engine::StratEngine;


/// Verify that tearing down an engine doesn't fail if no filesystems on it.
pub fn test_teardown(paths: &[&Path]) -> () {
    let mut engine = StratEngine::initialize().unwrap();
    engine
        .create_pool("test_pool", paths, None, true)
        .unwrap();
    assert!(engine.teardown().is_ok())
}


/// Verify that a pool with no devices does not have the minimum amount of
/// space required.
pub fn test_empty_pool(paths: &[&Path]) -> () {
    assert!(paths.len() == 0);
    let mut engine = StratEngine::initialize().unwrap();
    assert!(match engine
                      .create_pool("test_pool", paths, None, true)
                      .unwrap_err() {
                EngineError::Engine(ErrorEnum::Invalid, _) => true,
                _ => false,
            });
}

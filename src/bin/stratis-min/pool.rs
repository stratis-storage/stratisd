// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;

use libudev::{Context, Monitor};

use libstratis::{
    engine::{Engine, StratEngine},
    stratis::StratisResult,
};

use crate::print_table;

// stratis-min pool setup
pub fn pool_setup() -> StratisResult<()> {
    let mut engine = StratEngine::initialize()?;

    let ctxt = Context::new()?;
    let mtr = Monitor::new(&ctxt)?;
    let mut sock = mtr.listen()?;

    for uuid in engine.locked_pools().keys() {
        engine.unlock_pool(*uuid)?;
    }

    while let Some(event) = sock.receive_event() {
        engine.handle_event(&event);
    }

    Ok(())
}

// stratis-min pool create
pub fn pool_create(
    name: &str,
    blockdev_paths: &[&Path],
    key_desc: Option<String>,
) -> StratisResult<()> {
    let mut engine = StratEngine::initialize()?;
    engine.create_pool(name, blockdev_paths, None, key_desc)?;
    Ok(())
}

// stratis-min pool [list]
pub fn pool_list() -> StratisResult<()> {
    let engine = StratEngine::initialize()?;

    let pools = engine.pools();
    let name_col: Vec<_> = pools.iter().map(|(n, _, _)| n.to_string()).collect();
    let uuid_col: Vec<_> = pools
        .iter()
        .map(|(_, u, _)| u.to_simple_ref().to_string())
        .collect();
    let encrypted_col: Vec<_> = pools
        .iter()
        .map(|(_, _, p)| p.is_encrypted().to_string())
        .collect();
    print_table!(
        "Pool name", name_col;
        "UUID", uuid_col;
        "Encrypted", encrypted_col
    );

    Ok(())
}

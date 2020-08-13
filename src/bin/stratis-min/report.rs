// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use serde_json::Value;

use libstratis::{engine::StratEngine, stratis::StratisResult};

pub fn report() -> StratisResult<()> {
    let engine = StratEngine::initialize()?;
    let json: Value = (&engine).into();
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}

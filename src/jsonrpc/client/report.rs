// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use serde_json::Value;

use crate::{do_request, jsonrpc::interface::Stratis, stratis::StratisResult};

pub fn report() -> StratisResult<Value> {
    Ok(do_request!(Stratis::report))
}

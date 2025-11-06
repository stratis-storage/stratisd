// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use crate::{
    engine::{Engine, LockedPoolsInfo},
    stratis::VERSION,
};

pub fn version_prop() -> &'static str {
    VERSION
}

pub async fn locked_pools_prop(engine: &Arc<dyn Engine>) -> LockedPoolsInfo {
    engine.locked_pools().await
}

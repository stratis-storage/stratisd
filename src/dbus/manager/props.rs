// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use crate::{
    engine::{Engine, StoppedPoolsInfo},
    stratis::VERSION,
};

pub fn version_prop() -> &'static str {
    VERSION
}

pub async fn stopped_pools_prop(engine: &Arc<dyn Engine>) -> StoppedPoolsInfo {
    engine.stopped_pools().await
}

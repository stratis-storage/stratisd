// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use serde_json::Value;

use crate::engine::{Engine, Lockable};

#[inline]
pub async fn report(engine: Lockable<dyn Engine>) -> Value {
    engine.read().await.engine_state_report()
}

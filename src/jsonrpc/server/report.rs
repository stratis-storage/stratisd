// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use serde_json::Value;

use crate::engine::Engine;

#[inline]
pub async fn report<E>(engine: Arc<E>) -> Value
where
    E: Engine,
{
    engine.engine_state_report()
}

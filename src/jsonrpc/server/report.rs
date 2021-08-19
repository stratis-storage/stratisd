// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use serde_json::Value;

use crate::engine::{Engine, LockableEngine};

#[inline]
pub async fn report<E>(engine: LockableEngine<E>) -> Value
where
    E: Engine,
{
    engine.lock().await.engine_state_report()
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::Mutex;

use crate::engine::{Engine, ReportType};

#[inline]
pub async fn report(engine: Arc<Mutex<dyn Engine>>) -> Value {
    engine.lock().await.get_report(ReportType::EngineState)
}

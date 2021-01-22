// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{atomic::AtomicBool, Arc};

use futures::pending;
use tokio::sync::{mpsc::Receiver, Mutex};

use crate::{
    engine::{Engine, UdevEngineEvent},
    stratis::errors::StratisResult,
};

pub async fn setup(
    _engine: Arc<Mutex<dyn Engine>>,
    _recv: Receiver<UdevEngineEvent>,
    _should_exit: Arc<AtomicBool>,
) -> StratisResult<()> {
    Ok(pending!())
}

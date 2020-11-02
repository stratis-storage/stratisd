// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{mpsc::Receiver, Arc, Mutex};

use futures_util::pending;

use crate::engine::{Engine, UdevEngineEvent};

pub async fn setup(_engine: Arc<Mutex<dyn Engine>>, _recv: Receiver<UdevEngineEvent>) {
    pending!()
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{atomic::AtomicBool, Arc};

use tokio::{
    select,
    sync::{mpsc::Receiver, Mutex},
    task::JoinHandle,
};

use crate::{
    engine::{Engine, UdevEngineEvent},
    jsonrpc::run_server,
    stratis::{StratisError, StratisResult},
};

fn handle_udev(
    engine: Arc<Mutex<dyn Engine>>,
    mut recv: Receiver<UdevEngineEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let udev_event = match recv.recv().await {
                Some(u) => u,
                None => {
                    error!("Channel from udev handler to JSON RPC handler was shut");
                    return;
                }
            };
            let mut lock = engine.lock().await;
            let _ = lock.handle_event(&udev_event);
        }
    })
}

pub async fn setup(
    engine: Arc<Mutex<dyn Engine>>,
    recv: Receiver<UdevEngineEvent>,
    _should_exit: Arc<AtomicBool>,
) -> StratisResult<()> {
    let mut udev_join = handle_udev(Arc::clone(&engine), recv);
    let mut server_join = run_server(engine);

    select! {
        res = &mut udev_join => {
            error!("The JSON RPC udev handling thread exited...");
            res.map_err(|e| StratisError::Error(e.to_string()))
        }
        res = &mut server_join => {
            error!("The server handler thread exited...");
            res.map_err(|e| StratisError::Error(e.to_string()))
        }
    }
}

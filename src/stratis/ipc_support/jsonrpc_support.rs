// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use tokio::{
    select,
    sync::{broadcast::Sender, mpsc::UnboundedReceiver},
    task::JoinHandle,
};

use crate::{
    engine::{LockableEngine, UdevEngineEvent},
    jsonrpc::run_server,
    stratis::{StratisError, StratisResult},
};

fn handle_udev(
    engine: LockableEngine,
    mut recv: UnboundedReceiver<UdevEngineEvent>,
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
            // Return value should be ignored as JSON RPC does not keep a record
            // of data structure information in the IPC layer.
            let _ = lock.handle_event(&udev_event);
        }
    })
}

pub async fn setup(
    engine: LockableEngine,
    recv: UnboundedReceiver<UdevEngineEvent>,
    _: Sender<()>,
) -> StratisResult<()> {
    let mut udev_join = handle_udev(engine.clone(), recv);
    let mut server_join = run_server(engine);

    select! {
        res = &mut udev_join => {
            error!("The JSON RPC udev handling thread exited...");
            res.map_err(StratisError::from)
        }
        res = &mut server_join => {
            error!("The server handler thread exited...");
            res.map_err(StratisError::from)
        }
    }
}

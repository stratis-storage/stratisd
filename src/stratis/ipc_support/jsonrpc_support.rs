// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{sync::Arc, task::Poll};

use futures::{pin_mut, poll};
use tokio::{select, sync::mpsc::UnboundedReceiver, task::JoinHandle};

use crate::{
    engine::{Engine, UdevEngineEvent},
    jsonrpc::run_server,
    stratis::{StratisError, StratisResult},
};

fn handle_udev<E>(engine: Arc<E>, mut recv: UnboundedReceiver<UdevEngineEvent>) -> JoinHandle<()>
where
    E: 'static + Engine,
{
    tokio::spawn(async move {
        loop {
            let mut events = Vec::new();
            match recv.recv().await {
                Some(u) => events.push(u),
                None => {
                    error!("Channel from udev handler to JSON RPC handler was shut");
                    return;
                }
            };
            loop {
                let recv = recv.recv();
                pin_mut!(recv);
                match poll!(recv) {
                    Poll::Ready(Some(event)) => events.push(event),
                    Poll::Ready(None) => {
                        error!("Channel from udev handler to JSON RPC handler was shut");
                        return;
                    }
                    Poll::Pending => break,
                }
            }
            // Return value should be ignored as JSON RPC does not keep a record
            // of data structure information in the IPC layer.
            let _ = engine.handle_events(events).await;
        }
    })
}

pub async fn setup<E>(engine: Arc<E>, recv: UnboundedReceiver<UdevEngineEvent>) -> StratisResult<()>
where
    E: 'static + Engine,
{
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

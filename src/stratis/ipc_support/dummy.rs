// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{sync::Arc, task::Poll};

use futures::{pin_mut, poll};
use tokio::{spawn, sync::mpsc::UnboundedReceiver};

use crate::{
    engine::{Engine, UdevEngineEvent},
    stratis::{StratisError, StratisResult},
};

pub async fn setup(
    engine: Arc<dyn Engine>,
    mut receiver: UnboundedReceiver<UdevEngineEvent>,
) -> StratisResult<()> {
    spawn(async move {
        loop {
            let mut events = Vec::new();
            match receiver.recv().await {
                Some(u) => events.push(u),
                None => {
                    return Err(StratisError::Msg(
                        "Channel from udev handler to dummy handler was shut".to_string(),
                    ));
                }
            };
            loop {
                let recv = receiver.recv();
                pin_mut!(recv);
                match poll!(recv) {
                    Poll::Ready(Some(event)) => events.push(event),
                    Poll::Ready(None) => {
                        return Err(StratisError::Msg(
                            "Channel from udev handler to dummy handler was shut".to_string(),
                        ));
                    }
                    Poll::Pending => break,
                }
            }
            // Return value should be ignored as dummy handler does not keep a record
            // of data structure information as it has no IPC layer.
            let _ = engine.handle_events(events).await;
        }
    })
    .await
    .map_err(|e| StratisError::Join(e))
    .and_then(|res| res)
}

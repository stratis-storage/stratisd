// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{sync::Arc, time::Duration};

use dbus::{
    blocking::SyncConnection,
    channel::{default_reply, MatchingReceiver, Sender},
    message::MatchRule,
};
use tokio::{
    sync::broadcast::{error::TryRecvError, Receiver},
    task::spawn_blocking,
};

use crate::{
    dbus_api::types::LockableTree,
    engine::Engine,
    stratis::{StratisError, StratisResult},
};

/// Handler for a D-Bus receiving connection.
/// stratisd has exactly one connection handler, but this handler spawns
/// a thread for every D-Bus method.
pub struct DbusConnectionHandler<E> {
    connection: Arc<SyncConnection>,
    tree: LockableTree<E>,
    should_exit: Receiver<()>,
}

impl<E> DbusConnectionHandler<E>
where
    E: 'static + Engine,
{
    pub(super) fn new(
        connection: Arc<SyncConnection>,
        tree: LockableTree<E>,
        should_exit: Receiver<()>,
    ) -> DbusConnectionHandler<E> {
        DbusConnectionHandler {
            connection,
            tree,
            should_exit,
        }
    }

    /// Handle a D-Bus action passed from a D-Bus connection.
    /// Spawn a new thread for every D-Bus method call.
    /// Every method call requires a read lock on the D-Bus tree.
    pub fn process_dbus_requests(&mut self) -> StratisResult<()> {
        let tree = self.tree.clone();
        let connection = Arc::clone(&self.connection);
        let _ = self.connection.start_receive(
            MatchRule::new_method_call(),
            Box::new(move |msg, _| {
                let cloned_tree = tree.clone();
                let cloned_connection = Arc::clone(&connection);
                spawn_blocking(move || {
                    let lock = cloned_tree.blocking_read();
                    if let Some(msgs) = lock.handle(&msg) {
                        for msg in msgs {
                            if cloned_connection.send(msg).is_err() {
                                warn!("Failed to send reply to D-Bus client");
                            }
                        }
                    } else {
                        let reply = default_reply(&msg);
                        if let Some(r) = reply {
                            if cloned_connection.send(r).is_err() {
                                warn!("Failed to send reply to D-Bus client");
                            }
                        }
                    }
                });
                true
            }),
        );
        loop {
            if let Err(e) = self.connection.process(Duration::from_millis(100)) {
                warn!("Failed to process D-Bus request: {}", e);
            }
            match self.should_exit.try_recv() {
                Ok(()) => {
                    info!("D-Bus connection handler thread notified to exit");
                    break;
                }
                Err(TryRecvError::Lagged(_)) | Err(TryRecvError::Closed) => {
                    return Err(StratisError::Msg(
                        "D-Bus connection handler can't be notified to exit; shutting down..."
                            .to_string(),
                    ));
                }
                _ => (),
            }
        }

        Ok(())
    }
}

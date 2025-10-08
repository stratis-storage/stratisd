// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use dbus::{
    channel::{default_reply, MatchingReceiver, Sender},
    message::MatchRule,
    nonblock::SyncConnection,
};
use tokio::{task::spawn_blocking};

use crate::{dbus_api::types::LockableTree, stratis::StratisResult};

/// Handler for D-Bus messages sent to the daemon.
/// stratisd has exactly one message handler, but this handler spawns
/// a thread for every D-Bus method.
pub struct DbusMessageHandler {
    connection: Arc<SyncConnection>,
    tree: LockableTree,
}

impl DbusMessageHandler {
    pub(super) fn new(
        connection: Arc<SyncConnection>,
        tree: LockableTree,
    ) -> DbusMessageHandler {
        DbusMessageHandler { connection, tree }
    }

    /// Handle a D-Bus action passed from a D-Bus connection.
    /// Spawn a new thread for every D-Bus method call.
    /// Every method call requires a read lock on the D-Bus tree.
    pub async fn process_dbus_requests(&self) -> StratisResult<()> {
        let tree = self.tree.clone();
        let connection = self.connection.clone();
        let _ = self.connection.start_receive(
            MatchRule::new_method_call(),
            Box::new(move |msg, _| {
                let cloned_tree = tree.clone();
                let cloned_connection = connection.clone();
                spawn_blocking(move || {
                    trace!("Starting D-Bus request handling");
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
                    trace!("Finished D-Bus request handling");
                });
                true
            }),
        );
        Ok(())
    }
}

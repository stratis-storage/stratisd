// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use dbus::{channel::Sender, message::MatchRule, nonblock::SyncConnection};
use dbus_tree::{MTSync, Tree};
use futures::{executor::block_on, StreamExt};
use tokio::{
    sync::{mpsc::Receiver, RwLock},
    task::spawn_blocking,
};

use crate::{
    dbus_api::types::{DbusAction, TData},
    stratis::{StratisError, StratisResult},
};

/// Handler for a D-Bus tree. This will be used to process add and remove requests from
/// the tree.
pub struct DbusTreeHandler {
    tree: Arc<RwLock<Tree<MTSync<TData>, TData>>>,
    receiver: Receiver<DbusAction>,
}

impl DbusTreeHandler {
    pub fn new(
        tree: Arc<RwLock<Tree<MTSync<TData>, TData>>>,
        receiver: Receiver<DbusAction>,
    ) -> Self {
        DbusTreeHandler { tree, receiver }
    }

    /// Process a D-Bus action (add/remove) request.
    pub async fn process_dbus_actions(&mut self) -> StratisResult<()> {
        loop {
            let action = self.receiver.recv().await.ok_or_else(|| {
                StratisError::Error(
                    "The channel from the D-Bus request handler to the D-Bus object handler was closed".to_string()
                )
            })?;
            let mut write_lock = self.tree.write().await;
            match action {
                DbusAction::Add(path) => write_lock.insert(path),
                DbusAction::Remove(paths) => {
                    for path in paths {
                        write_lock.remove(&path);
                    }
                }
            }
        }
    }
}

/// Handler for a D-Bus receiving connection.
pub struct DbusConnectionHandler {
    connection: Arc<SyncConnection>,
    tree: Arc<RwLock<Tree<MTSync<TData>, TData>>>,
}

impl DbusConnectionHandler {
    pub(super) fn new(
        connection: Arc<SyncConnection>,
        tree: Arc<RwLock<Tree<MTSync<TData>, TData>>>,
    ) -> DbusConnectionHandler {
        DbusConnectionHandler { connection, tree }
    }

    /// Create a new reference to the D-Bus connection.
    pub fn new_connection_ref(&self) -> Arc<SyncConnection> {
        Arc::clone(&self.connection)
    }

    /// Handle a D-Bus action passed from a D-Bus connection.
    pub async fn process_dbus_requests(&self) -> StratisResult<()> {
        let match_msg = self
            .connection
            .add_match(MatchRule::new_method_call())
            .await?;
        let (_match_msg, mut stream) = match_msg.msg_stream();
        while let Some(msg) = stream.next().await {
            let tree = Arc::clone(&self.tree);
            let connection = Arc::clone(&self.connection);
            spawn_blocking(move || {
                let lock = block_on(tree.read());
                if let Some(msgs) = lock.handle(&msg) {
                    for msg in msgs {
                        if connection.send(msg).is_err() {
                            warn!("Failed to send reply to D-Bus client");
                        }
                    }
                }
            });
        }
        Ok(())
    }
}

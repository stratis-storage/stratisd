// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use tokio::{sync::mpsc::UnboundedReceiver, task::spawn_blocking};

use crate::{
    engine::{Engine, KeyDescription, PoolIdentifier},
    stratis::{StratisError, StratisResult},
};

/// Asynchronous task that receives key addition events and attempts to load the volume key for all
/// pools that may have been previously unopenable without this key in the kernel keyring.
pub async fn load_vks(
    engine: Arc<dyn Engine>,
    mut recv: Option<UnboundedReceiver<KeyDescription>>,
) -> StratisResult<()> {
    loop {
        if let Some(ref mut r) = recv {
            let sent_kd = r.recv().await.ok_or_else(|| {
                StratisError::Msg(
                    "Failed to receive updates in thread for processing key description additions"
                        .to_string(),
                )
            })?;
            let uuids = engine
                .pools()
                .await
                .iter()
                .filter_map(|(_, u, p)| {
                    if p.is_encrypted()
                        && p.encryption_info()
                            .and_then(|ei| {
                                ei.left()
                                    .map(|e| e.all_key_descriptions().any(|(_, kd)| kd == &sent_kd))
                            })
                            .unwrap_or(false)
                    {
                        Some(*u)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            for uuid in uuids {
                if let Some(mut p) = engine.get_mut_pool(PoolIdentifier::Uuid(uuid)).await {
                    match spawn_blocking(move || p.load_volume_key(uuid)).await {
                        Ok(Ok(false)) => (),
                        Ok(Ok(true)) => {
                            info!("Loaded volume key into keyring for pool with UUID {uuid}");
                        }
                        Ok(Err(e)) => {
                            warn!(
                                "Failed to load volume key into keyring for pool with UUID {uuid}: {e}"
                            );
                        }
                        Err(e) => {
                            warn!("Failed to join thread: {e}");
                        }
                    }
                } else {
                    info!("Pool with UUID {uuid} not found");
                }
            }
        } else {
            futures::future::pending().await
        }
    }
}

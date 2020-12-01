// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use chrono::SecondsFormat;
use tokio::sync::Mutex;

use crate::{
    engine::{Engine, EngineAction},
    jsonrpc::{interface::FsListType, server::utils::name_to_uuid_and_pool},
    stratis::{StratisError, StratisResult},
};

// stratis-min filesystem create
pub async fn filesystem_create(
    engine: Arc<Mutex<dyn Engine>>,
    pool_name: &str,
    name: &str,
) -> StratisResult<bool> {
    let mut lock = engine.lock().await;
    let (pool_uuid, pool) = name_to_uuid_and_pool(&mut *lock, pool_name)
        .ok_or_else(|| StratisError::Error(format!("No pool named {} found", pool_name)))?;
    Ok(pool
        .create_filesystems(pool_uuid, &[(name, None)])?
        .is_changed())
}

// stratis-min filesystem [list]
pub async fn filesystem_list(engine: Arc<Mutex<dyn Engine>>) -> FsListType {
    let lock = engine.lock().await;
    lock.pools().into_iter().fold(
        (
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
        |mut acc, (name, _uuid, pool)| {
            for (fs_name, uuid, fs) in pool.filesystems() {
                acc.0.push(name.to_string());
                acc.1.push(fs_name.to_string());
                acc.2.push(fs.used().ok().map(|u| *u));
                acc.3
                    .push(fs.created().to_rfc3339_opts(SecondsFormat::Secs, true));
                acc.4.push(fs.devnode());
                acc.5.push(uuid);
            }
            acc
        },
    )
}

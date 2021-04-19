// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use chrono::SecondsFormat;
use tokio::task::block_in_place;

use crate::{
    engine::{Engine, EngineAction, Locked, Name},
    jsonrpc::{interface::FsListType, server::utils::name_to_uuid_and_pool},
    stratis::{StratisError, StratisResult},
};

// stratis-min filesystem create
pub async fn filesystem_create(
    engine: Locked<dyn Engine>,
    pool_name: &str,
    name: &str,
) -> StratisResult<bool> {
    let mut lock = engine.write().await;
    let (pool_uuid, pool) = name_to_uuid_and_pool(&mut *lock, pool_name)
        .ok_or_else(|| StratisError::Error(format!("No pool named {} found", pool_name)))?;
    block_in_place(|| {
        Ok(pool
            .create_filesystems(pool_name, pool_uuid, &[(name, None)])?
            .is_changed())
    })
}

// stratis-min filesystem [list]
pub async fn filesystem_list(engine: Locked<dyn Engine>) -> FsListType {
    let lock = engine.read().await;
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

// stratis-min filesystem destroy
pub async fn filesystem_destroy(
    engine: Locked<dyn Engine>,
    pool_name: &str,
    fs_name: &str,
) -> StratisResult<bool> {
    let mut lock = engine.write().await;
    let (_, pool) = name_to_uuid_and_pool(&mut *lock, pool_name)
        .ok_or_else(|| StratisError::Error(format!("No pool named {} found", pool_name)))?;
    let (uuid, _) = pool
        .get_filesystem_by_name(&Name::new(fs_name.to_string()))
        .ok_or_else(|| StratisError::Error(format!("No filesystem named {} found", fs_name)))?;
    block_in_place(|| Ok(pool.destroy_filesystems(pool_name, &[uuid])?.is_changed()))
}

// stratis-min filesystem rename
pub async fn filesystem_rename(
    engine: Locked<dyn Engine>,
    pool_name: &str,
    fs_name: &str,
    new_fs_name: &str,
) -> StratisResult<bool> {
    let mut lock = engine.write().await;
    let (_, pool) = name_to_uuid_and_pool(&mut *lock, pool_name)
        .ok_or_else(|| StratisError::Error(format!("No pool named {} found", pool_name)))?;
    let (uuid, _) = pool
        .get_filesystem_by_name(&Name::new(fs_name.to_string()))
        .ok_or_else(|| StratisError::Error(format!("No filesystem named {} found", fs_name)))?;
    block_in_place(|| {
        Ok(pool
            .rename_filesystem(pool_name, uuid, new_fs_name)?
            .is_changed())
    })
}

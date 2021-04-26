// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use chrono::SecondsFormat;

use crate::{
    engine::{Engine, EngineAction, Lockable, Name},
    jsonrpc::{interface::FsListType, server::utils::name_to_uuid_and_pool},
    stratis::{StratisError, StratisResult},
};

// stratis-min filesystem create
pub async fn filesystem_create(
    engine: Lockable<dyn Engine>,
    pool_name: String,
    name: String,
) -> StratisResult<bool> {
    let lock = engine.read().await;
    let (pool_uuid, pool) = name_to_uuid_and_pool(&*lock, &pool_name)
        .ok_or_else(|| StratisError::Error(format!("No pool named {} found", pool_name)))?;
    spawn_blocking!({
        let mut pool_lock = lock!(pool, write);
        pool_lock
            .create_filesystems(&pool_name, pool_uuid, &[(&name, None)])
            .map(|a| a.is_changed())
    })
}

// stratis-min filesystem [list]
pub async fn filesystem_list(engine: Lockable<dyn Engine>) -> FsListType {
    let lock = engine.read().await;
    let mut vecs = (
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    for (name, _, pool) in lock.pools() {
        for (fs_name, uuid, fs) in pool.read().await.filesystems() {
            vecs.0.push(name.to_string());
            vecs.1.push(fs_name.to_string());
            vecs.2.push(fs.used().ok().map(|u| *u));
            vecs.3
                .push(fs.created().to_rfc3339_opts(SecondsFormat::Secs, true));
            vecs.4.push(fs.devnode());
            vecs.5.push(uuid);
        }
    }
    vecs
}

// stratis-min filesystem destroy
pub async fn filesystem_destroy(
    engine: Lockable<dyn Engine>,
    pool_name: String,
    fs_name: &str,
) -> StratisResult<bool> {
    let lock = engine.read().await;
    let (_, pool) = name_to_uuid_and_pool(&*lock, &pool_name)
        .ok_or_else(|| StratisError::Error(format!("No pool named {} found", pool_name)))?;
    let (uuid, _) = pool
        .read()
        .await
        .get_filesystem_by_name(&Name::new(fs_name.to_string()))
        .ok_or_else(|| StratisError::Error(format!("No filesystem named {} found", fs_name)))?;
    spawn_blocking!({
        let mut pool_lock = lock!(pool, write);
        pool_lock
            .destroy_filesystems(&pool_name, &[uuid])
            .map(|a| a.is_changed())
    })
}

// stratis-min filesystem rename
pub async fn filesystem_rename(
    engine: Lockable<dyn Engine>,
    pool_name: String,
    fs_name: String,
    new_fs_name: String,
) -> StratisResult<bool> {
    let lock = engine.read().await;
    let (_, pool) = name_to_uuid_and_pool(&*lock, &pool_name)
        .ok_or_else(|| StratisError::Error(format!("No pool named {} found", pool_name)))?;
    let name = Name::new(fs_name);
    let (uuid, _) = pool
        .read()
        .await
        .get_filesystem_by_name(&name)
        .ok_or_else(|| StratisError::Error(format!("No filesystem named {} found", name)))?;
    spawn_blocking!({
        let mut pool_lock = lock!(pool, write);
        pool_lock
            .rename_filesystem(&pool_name, uuid, &new_fs_name)
            .map(|a| a.is_changed())
    })
}

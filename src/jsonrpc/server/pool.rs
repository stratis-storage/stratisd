// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{os::unix::io::RawFd, path::PathBuf};

use crate::{
    engine::{
        BlockDevTier, EncryptionInfo, Engine, EngineAction, Lockable, PoolUuid, UnlockMethod,
    },
    jsonrpc::{
        interface::PoolListType,
        server::{
            key::{key_get_desc, key_set},
            utils::name_to_uuid_and_pool,
        },
    },
    stratis::{StratisError, StratisResult},
};

// stratis-min pool unlock
pub async fn pool_unlock(
    engine: Lockable<dyn Engine>,
    unlock_method: UnlockMethod,
    pool_uuid: Option<PoolUuid>,
    prompt: Option<RawFd>,
) -> StratisResult<bool> {
    if let Some(uuid) = pool_uuid {
        if let (Some(fd), Some(kd)) = (prompt, key_get_desc(engine.clone(), uuid).await) {
            key_set(engine.clone(), &kd, fd).await?;
        }
    }

    match pool_uuid {
        Some(u) => spawn_blocking!({
            let mut lock = lock!(engine, write);
            lock.unlock_pool(u, unlock_method).map(|a| a.is_changed())
        }),
        None => spawn_blocking!({
            let mut lock = lock!(engine, write);
            let changed = lock
                .locked_pools()
                .into_iter()
                .fold(false, |acc, (uuid, _)| {
                    let res = lock.unlock_pool(uuid, unlock_method);
                    if let Ok(ok) = res {
                        acc || ok.is_changed()
                    } else {
                        acc
                    }
                });
            Ok(changed)
        }),
    }
}

// stratis-min pool create
pub async fn pool_create(
    engine: Lockable<dyn Engine>,
    name: String,
    blockdev_paths: Vec<PathBuf>,
    enc_info: EncryptionInfo,
) -> StratisResult<bool> {
    spawn_blocking!({
        let mut lock = lock!(engine, write);
        let paths = blockdev_paths
            .iter()
            .map(|p| p.as_path())
            .collect::<Vec<_>>();
        lock.create_pool(name.as_str(), paths.as_slice(), None, &enc_info)
            .map(|a| a.is_changed())
    })
}

// stratis-min pool destroy
pub async fn pool_destroy(engine: Lockable<dyn Engine>, name: String) -> StratisResult<bool> {
    spawn_blocking!({
        let mut lock = lock!(engine, write);
        let (uuid, _) = name_to_uuid_and_pool(&*lock, name.as_str())
            .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", name)))?;
        lock.destroy_pool(uuid).map(|a| a.is_changed())
    })
}

// stratis-min pool init-cache
pub async fn pool_init_cache(
    engine: Lockable<dyn Engine>,
    name: String,
    paths: Vec<PathBuf>,
) -> StratisResult<bool> {
    let lock = engine.read().await;
    let (uuid, pool) = name_to_uuid_and_pool(&*lock, &name)
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", name)))?;
    spawn_blocking!({
        let mut pool_lock = lock!(pool, write);
        pool_lock
            .init_cache(
                uuid,
                &name,
                paths.iter().map(|p| &**p).collect::<Vec<_>>().as_slice(),
            )
            .map(|a| a.is_changed())
    })
}

// stratis-min pool rename
pub async fn pool_rename(
    engine: Lockable<dyn Engine>,
    current_name: String,
    new_name: String,
) -> StratisResult<bool> {
    spawn_blocking!({
        let mut lock = lock!(engine, write);
        let (uuid, _) = name_to_uuid_and_pool(&*lock, &current_name).ok_or_else(|| {
            StratisError::Error(format!("No pool found with name {}", current_name))
        })?;
        lock.rename_pool(uuid, &new_name).map(|a| a.is_changed())
    })
}

// stratis-min pool add-data
pub async fn pool_add_data(
    engine: Lockable<dyn Engine>,
    name: String,
    blockdevs: Vec<PathBuf>,
) -> StratisResult<bool> {
    add_blockdevs(engine, name, blockdevs, BlockDevTier::Data).await
}

// stratis-min pool add-cache
pub async fn pool_add_cache(
    engine: Lockable<dyn Engine>,
    name: String,
    blockdevs: Vec<PathBuf>,
) -> StratisResult<bool> {
    add_blockdevs(engine, name, blockdevs, BlockDevTier::Cache).await
}

async fn add_blockdevs(
    engine: Lockable<dyn Engine>,
    name: String,
    blockdevs: Vec<PathBuf>,
    tier: BlockDevTier,
) -> StratisResult<bool> {
    let lock = engine.read().await;
    let (uuid, pool) = name_to_uuid_and_pool(&*lock, name.as_str())
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", name)))?;
    spawn_blocking!({
        let paths = blockdevs.iter().map(|p| p.as_path()).collect::<Vec<_>>();
        lock!(pool, write)
            .add_blockdevs(uuid, name.as_str(), paths.as_slice(), tier)
            .map(|a| a.is_changed())
    })
}

// stratis-min pool [list]
pub async fn pool_list(engine: Lockable<dyn Engine>) -> PoolListType {
    let lock = engine.read().await;
    let (mut name_vec, mut size_vec, mut pool_props_vec, mut uuid_vec) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (n, u, p) in lock.pools().iter() {
        let pool_lock = p.read().await;
        let (s, p) = (
            (
                *pool_lock.total_physical_size().bytes(),
                pool_lock.total_physical_used().ok().map(|u| *u.bytes()),
            ),
            (pool_lock.has_cache(), pool_lock.is_encrypted()),
        );
        name_vec.push(n.to_string());
        size_vec.push(s);
        pool_props_vec.push(p);
        uuid_vec.push(*u);
    }
    (name_vec, size_vec, pool_props_vec, uuid_vec)
}

// stratis-min pool is-encrypted
pub async fn pool_is_encrypted(
    engine: Lockable<dyn Engine>,
    uuid: PoolUuid,
) -> StratisResult<bool> {
    let lock = engine.read().await;
    if let Some((_, pool)) = lock.get_pool(uuid) {
        Ok(pool.read().await.is_encrypted())
    } else if lock.locked_pools().get(&uuid).is_some() {
        Ok(true)
    } else {
        Err(StratisError::Error(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

// stratis-min pool is-locked
pub async fn pool_is_locked(engine: Lockable<dyn Engine>, uuid: PoolUuid) -> StratisResult<bool> {
    let lock = engine.read().await;
    if lock.get_pool(uuid).is_some() {
        Ok(false)
    } else if lock.locked_pools().get(&uuid).is_some() {
        Ok(true)
    } else {
        Err(StratisError::Error(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

// stratis-min pool is-bound
pub async fn pool_is_bound(engine: Lockable<dyn Engine>, uuid: PoolUuid) -> StratisResult<bool> {
    let lock = engine.read().await;
    if let Some((_, pool)) = lock.get_pool(uuid) {
        Ok(pool.read().await.encryption_info().clevis_info.is_some())
    } else if let Some(info) = lock.locked_pools().get(&uuid) {
        Ok(info.info.clevis_info.is_some())
    } else {
        Err(StratisError::Error(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

// stratis-min pool has-passphrase
pub async fn pool_has_passphrase(
    engine: Lockable<dyn Engine>,
    uuid: PoolUuid,
) -> StratisResult<bool> {
    let lock = engine.read().await;
    if let Some((_, pool)) = lock.get_pool(uuid) {
        Ok(pool
            .read()
            .await
            .encryption_info()
            .key_description
            .is_some())
    } else if let Some(info) = lock.locked_pools().get(&uuid) {
        Ok(info.info.key_description.is_some())
    } else {
        Err(StratisError::Error(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

// stratis-min pool clevis-pin
pub async fn pool_clevis_pin(
    engine: Lockable<dyn Engine>,
    uuid: PoolUuid,
) -> StratisResult<Option<String>> {
    let lock = engine.read().await;
    if let Some((_, pool)) = lock.get_pool(uuid) {
        Ok(pool
            .read()
            .await
            .encryption_info()
            .clevis_info
            .as_ref()
            .map(|(pin, _)| pin.clone()))
    } else if let Some(info) = lock.locked_pools().get(&uuid) {
        Ok(info.info.clevis_info.as_ref().map(|(pin, _)| pin.clone()))
    } else {
        Err(StratisError::Error(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

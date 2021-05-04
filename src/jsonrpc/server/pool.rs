// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{os::unix::io::RawFd, path::PathBuf, sync::Arc};

use crate::{
    engine::{BlockDevTier, EncryptionInfo, EngineAction, EngineType, PoolUuid, UnlockMethod},
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
    engine: EngineType,
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
        Some(u) => engine
            .unlock_pool(u, unlock_method)
            .await
            .map(|a| a.is_changed()),
        None => {
            let mut changed = false;
            for (uuid, _) in engine.locked_pools().await {
                let res = engine.unlock_pool(uuid, unlock_method).await;
                if let Ok(ok) = res {
                    changed = changed || ok.is_changed();
                }
            }
            Ok(changed)
        }
    }
}

// stratis-min pool create
pub async fn pool_create(
    engine: EngineType,
    name: String,
    blockdev_paths: Vec<PathBuf>,
    enc_info: EncryptionInfo,
) -> StratisResult<bool> {
    let paths = blockdev_paths
        .iter()
        .map(|p| p.as_path())
        .collect::<Vec<_>>();
    engine
        .create_pool(name.as_str(), paths.as_slice(), None, &enc_info)
        .await
        .map(|a| a.is_changed())
}

// stratis-min pool destroy
pub async fn pool_destroy(engine: EngineType, name: String) -> StratisResult<bool> {
    let (uuid, _) = name_to_uuid_and_pool(Arc::clone(&engine), name.as_str())
        .await
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", name)))?;
    engine.destroy_pool(uuid).await.map(|a| a.is_changed())
}

// stratis-min pool init-cache
pub async fn pool_init_cache(
    engine: EngineType,
    name: String,
    paths: Vec<PathBuf>,
) -> StratisResult<bool> {
    let (uuid, pool) = name_to_uuid_and_pool(Arc::clone(&engine), &name)
        .await
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
    engine: EngineType,
    current_name: String,
    new_name: String,
) -> StratisResult<bool> {
    let (uuid, _) = name_to_uuid_and_pool(Arc::clone(&engine), &current_name)
        .await
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", current_name)))?;
    engine
        .rename_pool(uuid, &new_name)
        .await
        .map(|a| a.is_changed())
}

// stratis-min pool add-data
pub async fn pool_add_data(
    engine: EngineType,
    name: String,
    blockdevs: Vec<PathBuf>,
) -> StratisResult<bool> {
    add_blockdevs(engine, name, blockdevs, BlockDevTier::Data).await
}

// stratis-min pool add-cache
pub async fn pool_add_cache(
    engine: EngineType,
    name: String,
    blockdevs: Vec<PathBuf>,
) -> StratisResult<bool> {
    add_blockdevs(engine, name, blockdevs, BlockDevTier::Cache).await
}

async fn add_blockdevs(
    engine: EngineType,
    name: String,
    blockdevs: Vec<PathBuf>,
    tier: BlockDevTier,
) -> StratisResult<bool> {
    let (uuid, pool) = name_to_uuid_and_pool(Arc::clone(&engine), name.as_str())
        .await
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", name)))?;
    spawn_blocking!({
        let paths = blockdevs.iter().map(|p| p.as_path()).collect::<Vec<_>>();
        lock!(pool, write)
            .add_blockdevs(uuid, name.as_str(), paths.as_slice(), tier)
            .map(|a| a.is_changed())
    })
}

// stratis-min pool [list]
pub async fn pool_list(engine: EngineType) -> PoolListType {
    let (mut name_vec, mut size_vec, mut pool_props_vec, mut uuid_vec) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (n, u, p) in engine.pools().await.iter() {
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
pub async fn pool_is_encrypted(engine: EngineType, uuid: PoolUuid) -> StratisResult<bool> {
    if let Some((_, pool)) = engine.get_pool(uuid).await {
        Ok(pool.read().await.is_encrypted())
    } else if engine.locked_pools().await.get(&uuid).is_some() {
        Ok(true)
    } else {
        Err(StratisError::Error(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

// stratis-min pool is-locked
pub async fn pool_is_locked(engine: EngineType, uuid: PoolUuid) -> StratisResult<bool> {
    if engine.get_pool(uuid).await.is_some() {
        Ok(false)
    } else if engine.locked_pools().await.get(&uuid).is_some() {
        Ok(true)
    } else {
        Err(StratisError::Error(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

// stratis-min pool is-bound
pub async fn pool_is_bound(engine: EngineType, uuid: PoolUuid) -> StratisResult<bool> {
    if let Some((_, pool)) = engine.get_pool(uuid).await {
        Ok(pool.read().await.encryption_info().clevis_info.is_some())
    } else if let Some(info) = engine.locked_pools().await.get(&uuid) {
        Ok(info.info.clevis_info.is_some())
    } else {
        Err(StratisError::Error(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

// stratis-min pool has-passphrase
pub async fn pool_has_passphrase(engine: EngineType, uuid: PoolUuid) -> StratisResult<bool> {
    if let Some((_, pool)) = engine.get_pool(uuid).await {
        Ok(pool
            .read()
            .await
            .encryption_info()
            .key_description
            .is_some())
    } else if let Some(info) = engine.locked_pools().await.get(&uuid) {
        Ok(info.info.key_description.is_some())
    } else {
        Err(StratisError::Error(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

// stratis-min pool clevis-pin
pub async fn pool_clevis_pin(engine: EngineType, uuid: PoolUuid) -> StratisResult<Option<String>> {
    if let Some((_, pool)) = engine.get_pool(uuid).await {
        Ok(pool
            .read()
            .await
            .encryption_info()
            .clevis_info
            .as_ref()
            .map(|(pin, _)| pin.clone()))
    } else if let Some(info) = engine.locked_pools().await.get(&uuid) {
        Ok(info.info.clevis_info.as_ref().map(|(pin, _)| pin.clone()))
    } else {
        Err(StratisError::Error(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

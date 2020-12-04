// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{os::unix::io::RawFd, path::Path, sync::Arc};

use tokio::sync::Mutex;

use crate::{
    engine::{
        BlockDevTier, CreateAction, DeleteAction, Engine, EngineAction, KeyDescription, PoolUuid,
        RenameAction, UnlockMethod,
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
    engine: Arc<Mutex<dyn Engine>>,
    pool_uuid: Option<PoolUuid>,
    prompt: Option<RawFd>,
) -> StratisResult<bool> {
    if let Some(uuid) = pool_uuid {
        if let (Some(fd), Some(kd)) = (prompt, key_get_desc(Arc::clone(&engine), uuid).await) {
            key_set(Arc::clone(&engine), &kd, fd).await?;
        }
    }

    let mut lock = engine.lock().await;
    match pool_uuid {
        Some(u) => Ok(lock
            .unlock_pool(u, UnlockMethod::Keyring)?
            .changed()
            .is_some()),
        None => {
            let changed = lock
                .locked_pools()
                .into_iter()
                .fold(false, |acc, (uuid, _)| {
                    let res = lock.unlock_pool(uuid, UnlockMethod::Keyring);
                    if let Ok(ok) = res {
                        acc || ok.is_changed()
                    } else {
                        acc
                    }
                });
            Ok(changed)
        }
    }
}

// stratis-min pool create
pub async fn pool_create(
    engine: Arc<Mutex<dyn Engine>>,
    name: &str,
    blockdev_paths: &[&Path],
    key_desc: Option<KeyDescription>,
) -> StratisResult<bool> {
    Ok(
        match engine
            .lock()
            .await
            .create_pool(name, blockdev_paths, None, key_desc)?
        {
            CreateAction::Created(_) => true,
            CreateAction::Identity => false,
        },
    )
}

// stratis-min pool destroy
pub async fn pool_destroy(engine: Arc<Mutex<dyn Engine>>, name: &str) -> StratisResult<bool> {
    let mut lock = engine.lock().await;
    let (uuid, _) = name_to_uuid_and_pool(&mut *lock, name)
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", name)))?;
    Ok(match lock.destroy_pool(uuid)? {
        DeleteAction::Deleted(_) => true,
        DeleteAction::Identity => false,
    })
}

// stratis-min pool init-cache
pub async fn pool_init_cache(
    engine: Arc<Mutex<dyn Engine>>,
    name: &str,
    paths: &[&Path],
) -> StratisResult<bool> {
    let mut lock = engine.lock().await;
    let (uuid, pool) = name_to_uuid_and_pool(&mut *lock, name)
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", name)))?;
    Ok(pool.init_cache(uuid, name, paths)?.is_changed())
}

// stratis-min pool rename
pub async fn pool_rename(
    engine: Arc<Mutex<dyn Engine>>,
    current_name: &str,
    new_name: &str,
) -> StratisResult<bool> {
    let mut lock = engine.lock().await;
    let (uuid, _) = name_to_uuid_and_pool(&mut *lock, current_name)
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", current_name)))?;
    Ok(match lock.rename_pool(uuid, new_name)? {
        RenameAction::Identity => false,
        RenameAction::Renamed(_) => true,
        RenameAction::NoSource => unreachable!(),
    })
}

// stratis-min pool add-data
pub async fn pool_add_data(
    engine: Arc<Mutex<dyn Engine>>,
    name: &str,
    blockdevs: &[&Path],
) -> StratisResult<bool> {
    add_blockdevs(engine, name, blockdevs, BlockDevTier::Data).await
}

// stratis-min pool add-cache
pub async fn pool_add_cache(
    engine: Arc<Mutex<dyn Engine>>,
    name: &str,
    blockdevs: &[&Path],
) -> StratisResult<bool> {
    add_blockdevs(engine, name, blockdevs, BlockDevTier::Cache).await
}

async fn add_blockdevs(
    engine: Arc<Mutex<dyn Engine>>,
    name: &str,
    blockdevs: &[&Path],
    tier: BlockDevTier,
) -> StratisResult<bool> {
    let mut lock = engine.lock().await;
    let (uuid, pool) = name_to_uuid_and_pool(&mut *lock, name)
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", name)))?;
    Ok(pool
        .add_blockdevs(uuid, name, blockdevs, tier)?
        .is_changed())
}

// stratis-min pool [list]
pub async fn pool_list(engine: Arc<Mutex<dyn Engine>>) -> PoolListType {
    let lock = engine.lock().await;
    let pools = lock.pools();
    (
        pools.iter().map(|(n, _, _)| n.to_string()).collect(),
        pools
            .iter()
            .map(|(_, _, p)| {
                (
                    *p.total_physical_size(),
                    p.total_physical_used().ok().map(|u| *u),
                )
            })
            .collect(),
        pools
            .iter()
            .map(|(_, _, p)| (p.has_cache(), p.is_encrypted()))
            .collect(),
    )
}

// stratis-min pool is-encrypted
pub async fn pool_is_encrypted(
    engine: Arc<Mutex<dyn Engine>>,
    uuid: PoolUuid,
) -> StratisResult<bool> {
    let lock = engine.lock().await;
    if let Some((_, pool)) = lock.get_pool(uuid) {
        Ok(pool.is_encrypted())
    } else if lock.locked_pools().get(&uuid).is_some() {
        Ok(true)
    } else {
        Err(StratisError::Error(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

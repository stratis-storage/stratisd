// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{convert::TryFrom, os::unix::io::RawFd, path::Path};

use tokio::task::block_in_place;

use crate::{
    engine::{
        BlockDevTier, CreateAction, DeleteAction, EncryptionInfo, Engine, EngineAction,
        LockableEngine, Pool, PoolUuid, RenameAction, UnlockMethod,
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
pub async fn pool_unlock<E>(
    engine: LockableEngine<E>,
    unlock_method: UnlockMethod,
    pool_uuid: Option<PoolUuid>,
    prompt: Option<RawFd>,
) -> StratisResult<bool>
where
    E: Engine,
{
    if let Some(uuid) = pool_uuid {
        if let (Some(fd), Some(kd)) = (prompt, key_get_desc(engine.clone(), uuid).await?) {
            key_set(engine.clone(), &kd, fd).await?;
        }
    }

    let mut lock = engine.lock().await;
    match pool_uuid {
        Some(u) => block_in_place(|| Ok(lock.unlock_pool(u, unlock_method)?.changed().is_some())),
        None => {
            let changed = lock
                .locked_pools()
                .into_iter()
                .fold(false, |acc, (uuid, _)| {
                    let res = block_in_place(|| lock.unlock_pool(uuid, unlock_method));
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
pub async fn pool_create<E>(
    engine: LockableEngine<E>,
    name: &str,
    blockdev_paths: &[&Path],
    enc_info: Option<&EncryptionInfo>,
) -> StratisResult<bool>
where
    E: Engine,
{
    let mut lock = engine.lock().await;
    Ok(
        match block_in_place(|| lock.create_pool(name, blockdev_paths, enc_info))? {
            CreateAction::Created(_) => true,
            CreateAction::Identity => false,
        },
    )
}

// stratis-min pool destroy
pub async fn pool_destroy<E>(engine: LockableEngine<E>, name: &str) -> StratisResult<bool>
where
    E: Engine,
{
    let mut lock = engine.lock().await;
    let (uuid, _) = name_to_uuid_and_pool(&mut *lock, name)
        .ok_or_else(|| StratisError::Msg(format!("No pool found with name {}", name)))?;
    Ok(match block_in_place(|| lock.destroy_pool(uuid))? {
        DeleteAction::Deleted(_) => true,
        DeleteAction::Identity => false,
    })
}

// stratis-min pool init-cache
pub async fn pool_init_cache<E>(
    engine: LockableEngine<E>,
    name: &str,
    paths: &[&Path],
) -> StratisResult<bool>
where
    E: Engine,
{
    let mut lock = engine.lock().await;
    let (uuid, pool) = name_to_uuid_and_pool(&mut *lock, name)
        .ok_or_else(|| StratisError::Msg(format!("No pool found with name {}", name)))?;
    block_in_place(|| Ok(pool.init_cache(uuid, name, paths)?.is_changed()))
}

// stratis-min pool rename
pub async fn pool_rename<E>(
    engine: LockableEngine<E>,
    current_name: &str,
    new_name: &str,
) -> StratisResult<bool>
where
    E: Engine,
{
    let mut lock = engine.lock().await;
    let (uuid, _) = name_to_uuid_and_pool(&mut *lock, current_name)
        .ok_or_else(|| StratisError::Msg(format!("No pool found with name {}", current_name)))?;
    Ok(match block_in_place(|| lock.rename_pool(uuid, new_name))? {
        RenameAction::Identity => false,
        RenameAction::Renamed(_) => true,
        RenameAction::NoSource => unreachable!(),
    })
}

// stratis-min pool add-data
pub async fn pool_add_data<E>(
    engine: LockableEngine<E>,
    name: &str,
    blockdevs: &[&Path],
) -> StratisResult<bool>
where
    E: Engine,
{
    add_blockdevs(engine, name, blockdevs, BlockDevTier::Data).await
}

// stratis-min pool add-cache
pub async fn pool_add_cache<E>(
    engine: LockableEngine<E>,
    name: &str,
    blockdevs: &[&Path],
) -> StratisResult<bool>
where
    E: Engine,
{
    add_blockdevs(engine, name, blockdevs, BlockDevTier::Cache).await
}

async fn add_blockdevs<E>(
    engine: LockableEngine<E>,
    name: &str,
    blockdevs: &[&Path],
    tier: BlockDevTier,
) -> StratisResult<bool>
where
    E: Engine,
{
    let mut lock = engine.lock().await;
    let (uuid, pool) = name_to_uuid_and_pool(&mut *lock, name)
        .ok_or_else(|| StratisError::Msg(format!("No pool found with name {}", name)))?;
    block_in_place(|| {
        Ok(pool
            .add_blockdevs(uuid, name, blockdevs, tier)?
            .is_changed())
    })
}

// stratis-min pool [list]
pub async fn pool_list<E>(engine: LockableEngine<E>) -> PoolListType
where
    E: Engine,
{
    let lock = engine.lock().await;
    lock.pools()
        .iter()
        .map(|(n, u, p)| {
            (
                n.to_string(),
                (
                    *p.total_physical_size().bytes(),
                    p.total_physical_used().ok().map(|u| *u.bytes()),
                ),
                (p.has_cache(), p.is_encrypted()),
                u,
            )
        })
        .fold(
            (Vec::new(), Vec::new(), Vec::new(), Vec::new()),
            |(mut name_vec, mut size_vec, mut pool_props_vec, mut uuid_vec), (n, s, p, u)| {
                name_vec.push(n);
                size_vec.push(s);
                pool_props_vec.push(p);
                uuid_vec.push(*u);
                (name_vec, size_vec, pool_props_vec, uuid_vec)
            },
        )
}

// stratis-min pool is-encrypted
pub async fn pool_is_encrypted<E>(engine: LockableEngine<E>, uuid: PoolUuid) -> StratisResult<bool>
where
    E: Engine,
{
    let lock = engine.lock().await;
    if let Some((_, pool)) = lock.get_pool(uuid) {
        Ok(pool.is_encrypted())
    } else if lock.locked_pools().get(&uuid).is_some() {
        Ok(true)
    } else {
        Err(StratisError::Msg(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

// stratis-min pool is-locked
pub async fn pool_is_locked<E>(engine: LockableEngine<E>, uuid: PoolUuid) -> StratisResult<bool>
where
    E: Engine,
{
    let lock = engine.lock().await;
    if lock.get_pool(uuid).is_some() {
        Ok(false)
    } else if lock.locked_pools().get(&uuid).is_some() {
        Ok(true)
    } else {
        Err(StratisError::Msg(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

// stratis-min pool is-bound
pub async fn pool_is_bound<E>(engine: LockableEngine<E>, uuid: PoolUuid) -> StratisResult<bool>
where
    E: Engine,
{
    let lock = engine.lock().await;
    if let Some((_, pool)) = lock.get_pool(uuid) {
        Ok(match pool.encryption_info() {
            Some(ei) => ei.clevis_info()?.is_some(),
            None => false,
        })
    } else if let Some(info) = lock.locked_pools().get(&uuid) {
        Ok(info.info.clevis_info()?.is_some())
    } else {
        Err(StratisError::Msg(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

// stratis-min pool has-passphrase
pub async fn pool_has_passphrase<E>(
    engine: LockableEngine<E>,
    uuid: PoolUuid,
) -> StratisResult<bool>
where
    E: Engine,
{
    let lock = engine.lock().await;
    if let Some((_, pool)) = lock.get_pool(uuid) {
        Ok(match pool.encryption_info() {
            Some(ei) => ei.key_description()?.is_some(),
            None => false,
        })
    } else if let Some(info) = lock.locked_pools().get(&uuid) {
        Ok(info.info.key_description()?.is_some())
    } else {
        Err(StratisError::Msg(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

// stratis-min pool clevis-pin
pub async fn pool_clevis_pin<E>(
    engine: LockableEngine<E>,
    uuid: PoolUuid,
) -> StratisResult<Option<String>>
where
    E: Engine,
{
    let lock = engine.lock().await;
    if let Some((_, pool)) = lock.get_pool(uuid) {
        let encryption_info = match pool.encryption_info() {
            Some(ei) => EncryptionInfo::try_from(ei)?,
            None => return Ok(None),
        };
        Ok(encryption_info.clevis_info().map(|(pin, _)| pin.clone()))
    } else if let Some(info) = lock.locked_pools().get(&uuid) {
        Ok(info.info.clevis_info()?.map(|(pin, _)| pin.clone()))
    } else {
        Err(StratisError::Msg(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{os::unix::io::RawFd, path::Path, sync::Arc};

use either::Either;
use serde_json::Value;
use tokio::task::block_in_place;

use crate::{
    engine::{
        BlockDevTier, CreateAction, DeleteAction, Engine, EngineAction, InputEncryptionInfo,
        IntegritySpec, KeyDescription, Name, OptionalTokenSlotInput, PoolIdentifier, PoolUuid,
        RenameAction, StratSigblockVersion, TokenUnlockMethod,
    },
    jsonrpc::interface::PoolListType,
    stratis::{StratisError, StratisResult},
};

// stratis-min pool start
pub async fn pool_start(
    engine: Arc<dyn Engine>,
    id: PoolIdentifier<PoolUuid>,
    unlock_method: TokenUnlockMethod,
    prompt: Option<RawFd>,
) -> StratisResult<bool> {
    Ok(engine
        .start_pool(id, unlock_method, prompt)
        .await?
        .is_changed())
}

// stratis-min pool stop
pub async fn pool_stop(
    engine: Arc<dyn Engine>,
    id: PoolIdentifier<PoolUuid>,
) -> StratisResult<bool> {
    Ok(engine.stop_pool(id, true).await?.is_changed())
}

// stratis-min pool create
pub async fn pool_create<'a>(
    engine: Arc<dyn Engine>,
    name: &'a str,
    blockdev_paths: &'a [&'a Path],
    enc_info: Option<&'a InputEncryptionInfo>,
) -> StratisResult<bool> {
    Ok(
        match engine
            .create_pool(name, blockdev_paths, enc_info, IntegritySpec::default())
            .await?
        {
            CreateAction::Created(_) => true,
            CreateAction::Identity => false,
        },
    )
}

// stratis-min pool destroy
pub async fn pool_destroy(engine: Arc<dyn Engine>, name: &str) -> StratisResult<bool> {
    let uuid = engine
        .get_pool(PoolIdentifier::Name(Name::new(name.to_owned())))
        .await
        .map(|g| g.as_tuple().1)
        .ok_or_else(|| StratisError::Msg(format!("No pool named {name} found")))?;
    Ok(engine.destroy_pool(uuid).await?.is_changed())
}

// stratis-min pool init-cache
pub async fn pool_init_cache<'a>(
    engine: Arc<dyn Engine>,
    name: &'a str,
    paths: &'a [&'a Path],
) -> StratisResult<bool> {
    let mut guard = engine
        .get_mut_pool(PoolIdentifier::Name(Name::new(name.to_owned())))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool named {name} found")))?;
    let (_, uuid, pool) = guard.as_mut_tuple();
    block_in_place(|| Ok(pool.init_cache(uuid, name, paths, true)?.is_changed()))
}

// stratis-min pool rename
pub async fn pool_rename<'a>(
    engine: Arc<dyn Engine>,
    current_name: &'a str,
    new_name: &'a str,
) -> StratisResult<bool> {
    let uuid = engine
        .get_pool(PoolIdentifier::Name(Name::new(current_name.to_owned())))
        .await
        .map(|g| g.as_tuple().1)
        .ok_or_else(|| StratisError::Msg(format!("No pool named {current_name} found")))?;
    Ok(match engine.rename_pool(uuid, new_name).await? {
        RenameAction::Identity => false,
        RenameAction::Renamed(_) => true,
        RenameAction::NoSource => unreachable!(),
    })
}

// stratis-min pool add-data
pub async fn pool_add_data(
    engine: Arc<dyn Engine>,
    name: &str,
    blockdevs: &[&Path],
) -> StratisResult<bool> {
    add_blockdevs(engine, name, blockdevs, BlockDevTier::Data).await
}

// stratis-min pool add-cache
pub async fn pool_add_cache(
    engine: Arc<dyn Engine>,
    name: &str,
    blockdevs: &[&Path],
) -> StratisResult<bool> {
    add_blockdevs(engine, name, blockdevs, BlockDevTier::Cache).await
}

async fn add_blockdevs<'a>(
    engine: Arc<dyn Engine>,
    name: &'a str,
    blockdevs: &'a [&'a Path],
    tier: BlockDevTier,
) -> StratisResult<bool> {
    let mut guard = engine
        .get_mut_pool(PoolIdentifier::Name(Name::new(name.to_owned())))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool named {name} found")))?;
    let (_, uuid, pool) = guard.as_mut_tuple();
    block_in_place(|| {
        Ok(pool
            .add_blockdevs(uuid, name, blockdevs, tier)?
            .0
            .is_changed())
    })
}

// stratis-min pool [list]
pub async fn pool_list(engine: Arc<dyn Engine>) -> PoolListType {
    let guard = engine.pools().await;
    guard
        .iter()
        .map(|(n, u, p)| {
            (
                n.to_string(),
                (
                    *p.total_physical_size().bytes(),
                    p.total_physical_used().map(|u| *u.bytes()),
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

// stratis-min pool bind keyring
pub async fn pool_bind_keyring(
    engine: Arc<dyn Engine>,
    id: PoolIdentifier<PoolUuid>,
    token_slot: OptionalTokenSlotInput,
    key_desc: &KeyDescription,
) -> StratisResult<bool> {
    let mut guard = engine
        .get_mut_pool(id.clone())
        .await
        .ok_or_else(|| StratisError::Msg(format!("Pool with {id} not found")))?;

    let (_, _, pool) = guard.as_mut_tuple();
    match pool.bind_keyring(token_slot, key_desc)? {
        CreateAction::Created(_key) => Ok(true),
        CreateAction::Identity => Ok(false),
    }
}

// stratis-min pool bind nbde|tang|tpm2
pub async fn pool_bind_clevis<'a>(
    engine: Arc<dyn Engine>,
    id: PoolIdentifier<PoolUuid>,
    token_slot: OptionalTokenSlotInput,
    pin: &'a str,
    clevis_info: &'a Value,
) -> StratisResult<bool> {
    let mut guard = engine
        .get_mut_pool(id.clone())
        .await
        .ok_or_else(|| StratisError::Msg(format!("Pool with {id} not found")))?;

    let (_, _, pool) = guard.as_mut_tuple();
    match pool.bind_clevis(token_slot, pin, clevis_info)? {
        CreateAction::Created(_clevis) => Ok(true),
        CreateAction::Identity => Ok(false),
    }
}

// stratis-min pool unbind keyring
pub async fn pool_unbind_keyring(
    engine: Arc<dyn Engine>,
    id: PoolIdentifier<PoolUuid>,
    token_slot: Option<u32>,
) -> StratisResult<bool> {
    let mut guard = engine
        .get_mut_pool(id.clone())
        .await
        .ok_or_else(|| StratisError::Msg(format!("Pool with {id} not found")))?;

    let (_, _, pool) = guard.as_mut_tuple();
    match pool.unbind_keyring(token_slot)? {
        DeleteAction::Deleted(_key_desc) => Ok(true),
        DeleteAction::Identity => Ok(false),
    }
}

// stratis-min pool unbind clevis
pub async fn pool_unbind_clevis(
    engine: Arc<dyn Engine>,
    id: PoolIdentifier<PoolUuid>,
    token_slot: Option<u32>,
) -> StratisResult<bool> {
    let mut guard = engine
        .get_mut_pool(id.clone())
        .await
        .ok_or_else(|| StratisError::Msg(format!("Pool with {id} not found")))?;

    let (_, _, pool) = guard.as_mut_tuple();
    match pool.unbind_clevis(token_slot)? {
        DeleteAction::Deleted(_clevis) => Ok(true),
        DeleteAction::Identity => Ok(false),
    }
}

// stratis-min pool rebind keyring
pub async fn pool_rebind_keyring(
    engine: Arc<dyn Engine>,
    id: PoolIdentifier<PoolUuid>,
    token_slot: Option<u32>,
    key_desc: KeyDescription,
) -> StratisResult<bool> {
    let mut guard = engine
        .get_mut_pool(id.clone())
        .await
        .ok_or_else(|| StratisError::Msg(format!("Pool with {id} not found")))?;

    let (_, _, pool) = guard.as_mut_tuple();
    match pool.rebind_keyring(token_slot, &key_desc)? {
        RenameAction::Renamed(_key) => Ok(true),
        RenameAction::Identity => Ok(false),
        RenameAction::NoSource => Ok(false),
    }
}

// stratis-min pool rebind clevis
pub async fn pool_rebind_clevis(
    engine: Arc<dyn Engine>,
    id: PoolIdentifier<PoolUuid>,
    token_slot: Option<u32>,
) -> StratisResult<bool> {
    let mut guard = engine
        .get_mut_pool(id.clone())
        .await
        .ok_or_else(|| StratisError::Msg(format!("Pool with {id} not found")))?;

    let (_, _, pool) = guard.as_mut_tuple();
    pool.rebind_clevis(token_slot)?;
    Ok(true)
}

// stratis-min pool is-encrypted
pub async fn pool_is_encrypted(
    engine: Arc<dyn Engine>,
    id: PoolIdentifier<PoolUuid>,
) -> StratisResult<bool> {
    let stopped = engine.stopped_pools().await;
    let guard = engine.get_pool(id.clone()).await;
    if let Some((_, _, pool)) = guard.as_ref().map(|guard| guard.as_tuple()) {
        Ok(pool.is_encrypted())
    } else {
        let pool_uuid = match id {
            PoolIdentifier::Uuid(ref u) => u,
            PoolIdentifier::Name(ref n) => stopped
                .name_to_uuid
                .get(n)
                .ok_or_else(|| StratisError::Msg(format!("Could not find pool with name {n}")))?,
        };
        if let Some(poolinfo) = stopped
            .stopped
            .get(pool_uuid)
            .or_else(|| stopped.partially_constructed.get(pool_uuid))
        {
            match poolinfo
            .metadata_version
            .ok_or_else(|| StratisError::Msg("Found multiple metadata versions".to_string()))?
            {
                StratSigblockVersion::V1 => Ok(poolinfo.info.is_some()),
                StratSigblockVersion::V2 => Ok(poolinfo.features.as_ref().ok_or_else(|| {
                    StratisError::Msg("Pool reports metadata version V2 but not features are available for the stopped pool".to_string())
                })?.encryption),
            }
        } else {
            Err(StratisError::Msg(format!("Pool with {id} not found")))
        }
    }
}

// stratis-min pool is-stopped
pub async fn pool_is_stopped(
    engine: Arc<dyn Engine>,
    id: PoolIdentifier<PoolUuid>,
) -> StratisResult<bool> {
    let stopped = engine.stopped_pools().await;
    if engine.get_pool(id.clone()).await.is_some() {
        return Ok(false);
    }
    let pool_uuid = match id {
        PoolIdentifier::Uuid(ref u) => u,
        PoolIdentifier::Name(ref n) => stopped
            .name_to_uuid
            .get(n)
            .ok_or_else(|| StratisError::Msg(format!("Could not find pool with name {n}")))?,
    };
    if stopped
        .stopped
        .get(pool_uuid)
        .or_else(|| stopped.partially_constructed.get(pool_uuid))
        .is_some()
    {
        Ok(true)
    } else {
        Err(StratisError::Msg(format!("Pool with {id} not found")))
    }
}

// stratis-min pool has-passphrase
pub async fn pool_has_passphrase(
    engine: Arc<dyn Engine>,
    id: PoolIdentifier<PoolUuid>,
) -> StratisResult<bool> {
    let locked = engine.locked_pools().await;
    let guard = engine.get_pool(id.clone()).await;
    if let Some((_, _, pool)) = guard.as_ref().map(|guard| guard.as_tuple()) {
        Ok(match pool.encryption_info() {
            Some(Either::Left(ei)) => ei.all_key_descriptions().count() > 0,
            Some(Either::Right(ei)) => ei.key_description()?.is_some(),
            None => false,
        })
    } else if let Some(info) = locked.locked.get(match id {
        PoolIdentifier::Uuid(ref u) => u,
        PoolIdentifier::Name(ref n) => locked
            .name_to_uuid
            .get(n)
            .ok_or_else(|| StratisError::Msg(format!("Could not find pool with name {n}")))?,
    }) {
        Ok(info.info.key_description()?.is_some())
    } else {
        Err(StratisError::Msg(format!("Pool with {id} not found")))
    }
}

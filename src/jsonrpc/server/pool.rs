// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{os::unix::io::RawFd, path::Path};

use crate::{
    engine::{
        BlockDevTier, CreateAction, DeleteAction, Engine, EngineAction, KeyDescription, Pool,
        PoolUuid, RenameAction, StratEngine,
    },
    jsonrpc::{
        interface::PoolListType,
        server::key::{key_get_desc, key_set},
    },
    stratis::{StratisError, StratisResult},
};

// stratis-min pool unlock
pub fn pool_unlock(
    engine: &mut StratEngine,
    pool_uuid: PoolUuid,
    prompt: Option<(RawFd, bool)>,
) -> StratisResult<bool> {
    if let (Some((fd, no_tty)), Some(kd)) = (prompt, key_get_desc(engine, pool_uuid)) {
        key_set(engine, &kd, fd, Some(!no_tty))?;
    }

    Ok(engine.unlock_pool(pool_uuid)?.changed().is_some())
}

// stratis-min pool create
pub fn pool_create(
    engine: &mut StratEngine,
    name: &str,
    blockdev_paths: &[&Path],
    key_desc: Option<KeyDescription>,
) -> StratisResult<bool> {
    Ok(
        match engine.create_pool(name, blockdev_paths, None, key_desc)? {
            CreateAction::Created(_) => true,
            CreateAction::Identity => false,
        },
    )
}

// stratis-min pool destroy
pub fn pool_destroy(engine: &mut StratEngine, name: &str) -> StratisResult<bool> {
    let (uuid, _) = name_to_uuid_and_pool(engine, name)
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", name)))?;
    Ok(match engine.destroy_pool(uuid)? {
        DeleteAction::Deleted(_) => true,
        DeleteAction::Identity => false,
    })
}

/// Convert a string representing the name of a pool to the UUID and stratisd
/// data structure representing the pool state.
fn name_to_uuid_and_pool<'a>(
    engine: &'a mut StratEngine,
    name: &str,
) -> Option<(PoolUuid, &'a mut dyn Pool)> {
    let mut uuids_pools_for_name = engine
        .pools_mut()
        .into_iter()
        .filter_map(|(n, u, p)| if &*n == name { Some((u, p)) } else { None })
        .collect::<Vec<_>>();
    assert!(uuids_pools_for_name.len() <= 1);
    uuids_pools_for_name.pop()
}

// stratis-min pool init-cache
pub fn pool_init_cache(
    engine: &mut StratEngine,
    name: &str,
    paths: &[&Path],
) -> StratisResult<bool> {
    let (uuid, pool) = name_to_uuid_and_pool(engine, name)
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", name)))?;
    Ok(pool.init_cache(uuid, name, paths)?.is_changed())
}

// stratis-min pool rename
pub fn pool_rename(
    engine: &mut StratEngine,
    current_name: &str,
    new_name: &str,
) -> StratisResult<bool> {
    let (uuid, _) = name_to_uuid_and_pool(engine, current_name)
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", current_name)))?;
    Ok(match engine.rename_pool(uuid, new_name)? {
        RenameAction::Identity => false,
        RenameAction::Renamed(_) => true,
        RenameAction::NoSource => unreachable!(),
    })
}

// stratis-min pool add-data
pub fn pool_add_data(
    engine: &mut StratEngine,
    name: &str,
    blockdevs: &[&Path],
) -> StratisResult<bool> {
    add_blockdevs(engine, name, blockdevs, BlockDevTier::Data)
}

// stratis-min pool add-cache
pub fn pool_add_cache(
    engine: &mut StratEngine,
    name: &str,
    blockdevs: &[&Path],
) -> StratisResult<bool> {
    add_blockdevs(engine, name, blockdevs, BlockDevTier::Cache)
}

fn add_blockdevs(
    engine: &mut StratEngine,
    name: &str,
    blockdevs: &[&Path],
    tier: BlockDevTier,
) -> StratisResult<bool> {
    let (uuid, pool) = name_to_uuid_and_pool(engine, name)
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", name)))?;
    Ok(pool
        .add_blockdevs(uuid, name, blockdevs, tier)?
        .is_changed())
}

pub fn pool_list(engine: &mut StratEngine) -> PoolListType {
    let pools = engine.pools();
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
pub fn pool_is_encrypted(engine: &mut StratEngine, uuid: PoolUuid) -> StratisResult<bool> {
    if let Some((_, pool)) = engine.get_pool(uuid) {
        Ok(pool.is_encrypted())
    } else if engine.locked_pools().get(&uuid).is_some() {
        Ok(true)
    } else {
        Err(StratisError::Error(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )))
    }
}

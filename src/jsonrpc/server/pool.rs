// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{os::unix::io::RawFd, path::Path};

use crate::{
    engine::{
        BlockDevTier, CreateAction, DeleteAction, Engine, EngineAction, Pool, PoolUuid,
        RenameAction, StratEngine,
    },
    jsonrpc::{
        consts::{OP_ERR, OP_OK, OP_OK_STR},
        interface::PoolListType,
        server::key::{key_get_desc, key_set_internal},
        utils::stratis_error_to_return,
    },
    stratis::{StratisError, StratisResult},
};

// stratis-min pool unlock
pub fn pool_unlock(
    engine: &mut StratEngine,
    pool_uuid: PoolUuid,
    prompt: Option<(RawFd, bool)>,
) -> (bool, u16, String) {
    let default_value = false;
    if let Some((fd, no_tty)) = prompt {
        if let Some(kd) = key_get_desc(engine, pool_uuid) {
            if let Err(e) = key_set_internal(engine, kd, fd, Some(!no_tty)) {
                let (rc, rs) = stratis_error_to_return(e);
                return (default_value, rc, rs);
            }
        }
    }

    match engine
        .unlock_pool(pool_uuid)
        .map(|ret| ret.changed().is_some())
    {
        Ok(changed) => (changed, OP_OK, OP_OK_STR.to_string()),
        Err(e) => {
            let (rc, rs) = stratis_error_to_return(e);
            (default_value, rc, rs)
        }
    }
}

// stratis-min pool create
pub fn pool_create(
    engine: &mut StratEngine,
    name: &str,
    blockdev_paths: &[&Path],
    key_desc: Option<String>,
) -> (Option<PoolUuid>, u16, String) {
    match engine.create_pool(name, blockdev_paths, None, key_desc) {
        Ok(CreateAction::Created(uuid)) => (Some(uuid), OP_OK, OP_OK_STR.to_string()),
        Ok(CreateAction::Identity) => (None, OP_OK, OP_OK_STR.to_string()),
        Err(e) => {
            let (rc, rs) = stratis_error_to_return(e);
            (None, rc, rs)
        }
    }
}

// stratis-min pool destroy
pub fn pool_destroy(engine: &mut StratEngine, name: &str) -> (bool, u16, String) {
    let uuid = match name_to_uuid_and_pool(engine, name) {
        Some((u, _)) => u,
        None => return (false, OP_ERR, format!("No pool found with name {}", name)),
    };
    match engine.destroy_pool(uuid) {
        Ok(DeleteAction::Deleted(_)) => (true, OP_OK, OP_OK_STR.to_string()),
        Ok(DeleteAction::Identity) => (false, OP_OK, OP_OK_STR.to_string()),
        Err(e) => {
            let (rc, rs) = stratis_error_to_return(e);
            (false, rc, rs)
        }
    }
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
) -> (bool, u16, String) {
    let (uuid, pool) = match name_to_uuid_and_pool(engine, name) {
        Some(up) => up,
        None => {
            let (rc, rs) = stratis_error_to_return(StratisError::Error(format!(
                "No pool found with name {}",
                name
            )));
            return (false, rc, rs);
        }
    };
    match pool.init_cache(uuid, name, paths) {
        Ok(ret) => (ret.is_changed(), OP_OK, OP_OK_STR.to_string()),
        Err(e) => {
            let (rc, rs) = stratis_error_to_return(e);
            (false, rc, rs)
        }
    }
}

// stratis-min pool rename
pub fn pool_rename(
    engine: &mut StratEngine,
    current_name: &str,
    new_name: &str,
) -> (bool, u16, String) {
    let uuid = match name_to_uuid_and_pool(engine, current_name) {
        Some((u, _)) => u,
        None => {
            let (rc, rs) = stratis_error_to_return(StratisError::Error(format!(
                "No pool found with name {}",
                current_name
            )));
            return (false, rc, rs);
        }
    };
    match engine.rename_pool(uuid, new_name) {
        Ok(RenameAction::Identity) => (false, OP_OK, OP_OK_STR.to_string()),
        Ok(RenameAction::Renamed(_)) => (true, OP_OK, OP_OK_STR.to_string()),
        Ok(RenameAction::NoSource) => unreachable!(),
        Err(e) => {
            let (rc, rs) = stratis_error_to_return(e);
            (false, rc, rs)
        }
    }
}

// stratis-min pool add-data
pub fn pool_add_data(
    engine: &mut StratEngine,
    name: &str,
    blockdevs: &[&Path],
) -> (bool, u16, String) {
    match add_blockdevs(engine, name, blockdevs, BlockDevTier::Data) {
        Ok(ret) => (ret, OP_OK, OP_OK_STR.to_string()),
        Err(e) => {
            let (rc, rs) = stratis_error_to_return(e);
            (false, rc, rs)
        }
    }
}

// stratis-min pool add-cache
pub fn pool_add_cache(
    engine: &mut StratEngine,
    name: &str,
    blockdevs: &[&Path],
) -> (bool, u16, String) {
    match add_blockdevs(engine, name, blockdevs, BlockDevTier::Cache) {
        Ok(ret) => (ret, OP_OK, OP_OK_STR.to_string()),
        Err(e) => {
            let (rc, rs) = stratis_error_to_return(e);
            (false, rc, rs)
        }
    }
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
pub fn pool_is_encrypted(engine: &mut StratEngine, uuid: PoolUuid) -> (bool, u16, String) {
    if let Some((_, pool)) = engine.get_pool(uuid) {
        (pool.is_encrypted(), OP_OK, OP_OK_STR.to_string())
    } else if engine.locked_pools().get(&uuid).is_some() {
        (true, OP_OK, OP_OK_STR.to_string())
    } else {
        let (rc, rs) = stratis_error_to_return(StratisError::Error(format!(
            "Pool with UUID {} not found",
            uuid.to_simple_ref()
        )));
        (false, rc, rs)
    }
}

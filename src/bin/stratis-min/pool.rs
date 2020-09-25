// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;

use libudev::{Context, Monitor};

use libstratis::{
    engine::{
        BlockDevTier, CreateAction, DeleteAction, Engine, EngineAction, Pool, PoolUuid,
        RenameAction, StratEngine,
    },
    stratis::{StratisError, StratisResult},
};

use crate::{
    key::{key_get_desc, key_set},
    print_table,
};

const SUFFIXES: &[(u64, &str)] = &[
    (60, "EiB"),
    (50, "PiB"),
    (40, "TiB"),
    (30, "GiB"),
    (20, "MiB"),
    (10, "KiB"),
    (1, "B"),
];

/// Unlock one specific pool and fail if the pool cannot be unlocked.
#[inline]
fn unlock_one_pool(engine: &mut StratEngine, pool_uuid: PoolUuid) -> StratisResult<()> {
    engine.unlock_pool(pool_uuid).map(|_| ())
}

/// Attempt to unlock all locked pools and simply print a message if some pools fail to
/// unlock.
#[inline]
fn unlock_all_pools(engine: &mut StratEngine) {
    for uuid in engine.locked_pools().keys() {
        if let Err(e) = engine.unlock_pool(*uuid) {
            println!(
                "Could not unlock pool with UUID {}: {}",
                uuid.to_simple_ref(),
                e
            );
        }
    }
}

// stratis-min pool setup
pub fn pool_setup(pool_uuid: Option<PoolUuid>) -> StratisResult<()> {
    if let Some(uuid) = pool_uuid {
        let key_desc = key_get_desc(uuid)?;
        if let Some(ref kd) = key_desc {
            key_set(kd, None, true)?;
        }
    }

    let mut engine = StratEngine::initialize()?;

    let ctxt = Context::new()?;
    let mtr = Monitor::new(&ctxt)?;
    let mut sock = mtr.listen()?;

    match pool_uuid {
        Some(uuid) => unlock_one_pool(&mut engine, uuid)?,
        None => unlock_all_pools(&mut engine),
    }

    while let Some(event) = sock.receive_event() {
        engine.handle_event(&event);
    }

    Ok(())
}

// stratis-min pool create
pub fn pool_create(
    name: &str,
    blockdev_paths: &[&Path],
    key_desc: Option<String>,
) -> StratisResult<()> {
    let mut engine = StratEngine::initialize()?;
    if let CreateAction::Identity = engine.create_pool(name, blockdev_paths, None, key_desc)? {
        Err(StratisError::Error(format!(
            "Pool {} already exists as requested.",
            name
        )))
    } else {
        Ok(())
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

// stratis-min pool destroy
pub fn pool_destroy(name: &str) -> StratisResult<()> {
    let mut engine = StratEngine::initialize()?;
    let (uuid, _) = name_to_uuid_and_pool(&mut engine, name)
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", name)))?;
    if let DeleteAction::Identity = engine.destroy_pool(uuid)? {
        Err(StratisError::Error(format!(
            "Pool with name {} does not exist to be deleted.",
            name
        )))
    } else {
        Ok(())
    }
}

// stratis-min pool init-cache
pub fn pool_init_cache(name: &str, paths: &[&Path]) -> StratisResult<()> {
    let mut engine = StratEngine::initialize()?;
    let (uuid, pool) = name_to_uuid_and_pool(&mut engine, name)
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", name)))?;
    if pool.init_cache(uuid, name, paths)?.is_changed() {
        Ok(())
    } else {
        Err(StratisError::Error(
            "The cache has already been initialized as requested.".to_string(),
        ))
    }
}

// stratis-min pool rename
pub fn pool_rename(current_name: &str, new_name: &str) -> StratisResult<()> {
    let mut engine = StratEngine::initialize()?;
    let (uuid, _) = name_to_uuid_and_pool(&mut engine, current_name)
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", current_name)))?;
    match engine.rename_pool(uuid, new_name)? {
        RenameAction::Identity => Err(StratisError::Error(format!(
            "The selected pool is already named {}",
            current_name
        ))),
        RenameAction::NoSource => unreachable!(),
        _ => Ok(()),
    }
}

// stratis-min pool add-data
pub fn pool_add_data(name: &str, blockdevs: &[&Path]) -> StratisResult<()> {
    add_blockdevs(name, blockdevs, BlockDevTier::Data)
}

// stratis-min pool add-cache
pub fn pool_add_cache(name: &str, blockdevs: &[&Path]) -> StratisResult<()> {
    add_blockdevs(name, blockdevs, BlockDevTier::Cache)
}

fn add_blockdevs(name: &str, blockdevs: &[&Path], tier: BlockDevTier) -> StratisResult<()> {
    let mut engine = StratEngine::initialize()?;
    let (uuid, pool) = name_to_uuid_and_pool(&mut engine, name)
        .ok_or_else(|| StratisError::Error(format!("No pool found with name {}", name)))?;
    if pool
        .add_blockdevs(uuid, name, blockdevs, tier)?
        .is_changed()
    {
        Ok(())
    } else {
        Err(StratisError::Error(format!(
            "Pool {} already contains the given block devices",
            name
        )))
    }
}

#[allow(clippy::cast_precision_loss)]
fn to_suffix_repr(size: u64) -> String {
    SUFFIXES.iter().fold(String::new(), |acc, (div, suffix)| {
        let div_shifted = 1 << div;
        if acc.is_empty() && size / div_shifted >= 1 {
            format!(
                "{:.2} {}",
                (size / div_shifted) as f64 + ((size % div_shifted) as f64 / div_shifted as f64),
                suffix
            )
        } else {
            acc
        }
    })
}

fn size_string(p: &dyn Pool) -> String {
    let size = p.total_physical_size().bytes();
    let used = p.total_physical_used().ok().map(|u| u.bytes());
    let free = used.map(|u| size - u);
    format!(
        "{} / {} / {}",
        to_suffix_repr(*size),
        match used {
            Some(u) => to_suffix_repr(*u),
            None => "FAILURE".to_string(),
        },
        match free {
            Some(f) => to_suffix_repr(*f),
            None => "FAILURE".to_string(),
        }
    )
}

fn properties_string(p: &dyn Pool) -> String {
    let ca = if p.has_cache() { " Ca" } else { "~Ca" };
    let cr = if p.is_encrypted() { " Cr" } else { "~Cr" };
    vec![ca, cr].join(",")
}

// stratis-min pool [list]
pub fn pool_list() -> StratisResult<()> {
    let engine = StratEngine::initialize()?;

    let pools = engine.pools();
    let name_col: Vec<_> = pools.iter().map(|(n, _, _)| n.to_string()).collect();
    let physical_col: Vec<_> = pools.iter().map(|(_, _, p)| size_string(*p)).collect();
    let properties_col: Vec<_> = pools
        .iter()
        .map(|(_, _, p)| properties_string(*p))
        .collect();
    print_table!(
        "Name", name_col, "<";
        "Total Physical", physical_col, ">";
        "Properties", properties_col, ">"
    );

    Ok(())
}

// stratis-min pool is-encrypted
pub fn pool_is_encrypted(uuid: PoolUuid) -> StratisResult<bool> {
    let engine = StratEngine::initialize()?;
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

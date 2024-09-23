// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use chrono::SecondsFormat;
use tokio::task::block_in_place;

use crate::{
    engine::{Engine, EngineAction, Name, PoolIdentifier},
    jsonrpc::interface::FsListType,
    stratis::{StratisError, StratisResult},
};

// stratis-min filesystem create
pub async fn filesystem_create<'a>(
    engine: Arc<dyn Engine>,
    pool_name: &'a str,
    name: &'a str,
) -> StratisResult<bool> {
    let mut guard = engine
        .get_mut_pool(PoolIdentifier::Name(Name::new(pool_name.to_owned())))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool named {pool_name} found")))?;
    let (_, pool_uuid, pool) = guard.as_mut_tuple();
    block_in_place(|| {
        Ok(pool
            .create_filesystems(pool_name, pool_uuid, &[(name, None, None)])?
            .is_changed())
    })
}

// stratis-min filesystem [list]
pub async fn filesystem_list(engine: Arc<dyn Engine>) -> FsListType {
    let guard = engine.pools().await;
    guard.iter().fold(
        (
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
        |mut acc, (name, _, pool)| {
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
pub async fn filesystem_destroy<'a>(
    engine: Arc<dyn Engine>,
    pool_name: &'a str,
    fs_name: &'a str,
) -> StratisResult<bool> {
    let mut pool = engine
        .get_mut_pool(PoolIdentifier::Name(Name::new(pool_name.to_owned())))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool named {pool_name} found")))?;
    let (uuid, _) = pool
        .get_filesystem_by_name(&Name::new(fs_name.to_string()))
        .ok_or_else(|| StratisError::Msg(format!("No filesystem named {fs_name} found")))?;
    block_in_place(|| Ok(pool.destroy_filesystems(pool_name, &[uuid])?.is_changed()))
}

// stratis-min filesystem rename
pub async fn filesystem_rename<'a>(
    engine: Arc<dyn Engine>,
    pool_name: &'a str,
    fs_name: &'a str,
    new_fs_name: &'a str,
) -> StratisResult<bool> {
    let mut pool = engine
        .get_mut_pool(PoolIdentifier::Name(Name::new(pool_name.to_owned())))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool named {pool_name} found")))?;
    let (uuid, _) = pool
        .get_filesystem_by_name(&Name::new(fs_name.to_string()))
        .ok_or_else(|| StratisError::Msg(format!("No filesystem named {fs_name} found")))?;
    block_in_place(|| {
        Ok(pool
            .rename_filesystem(pool_name, uuid, new_fs_name)?
            .is_changed())
    })
}

// stratis-min filesystem origin
pub async fn filesystem_origin<'a>(
    engine: Arc<dyn Engine>,
    pool_name: &'a str,
    fs_name: &'a str,
) -> StratisResult<Option<String>> {
    let pool = engine
        .get_pool(PoolIdentifier::Name(Name::new(pool_name.to_owned())))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool named {pool_name} found")))?;
    let (_, fs) = pool
        .get_filesystem_by_name(&Name::new(fs_name.to_string()))
        .ok_or_else(|| StratisError::Msg(format!("No filesystem named {fs_name} found")))?;
    Ok(fs.origin().map(|u| u.as_simple().to_string()))
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use tokio::sync::RwLock;
use zbus::{zvariant::ObjectPath, Connection};

use crate::{
    dbus::{
        blockdev::unregister_blockdev, consts, filesystem::unregister_filesystem,
        register_blockdev, register_filesystem, Manager,
    },
    engine::{DevUuid, Engine, FilesystemUuid, Lockable, PoolIdentifier, PoolUuid},
    stratis::{StratisError, StratisResult},
};

mod interface;
mod methods;
mod props;
mod shared;

pub use interface::PoolInterface;

pub async fn register_pool<'a>(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    pool_uuid: PoolUuid,
) -> StratisResult<(ObjectPath<'a>, Vec<ObjectPath<'a>>, Vec<ObjectPath<'a>>)> {
    match engine.get_pool(PoolIdentifier::Uuid(pool_uuid)).await {
        Some(pool) => {
            let path = ObjectPath::try_from(format!(
                "{}/{}",
                consts::STRATIS_BASE_PATH,
                counter.fetch_add(1, Ordering::AcqRel),
            ))?;

            if let Err(e) = PoolInterface::register(
                engine,
                connection,
                manager,
                counter,
                path.clone(),
                pool_uuid,
            )
            .await
            {
                warn!("Failed to register interface pool for pool with UUID {pool_uuid}: {e}");
            }

            manager.write().await.add_pool(&path, pool_uuid)?;

            let mut fs_paths = Vec::new();
            let fs_uuids = pool.filesystems().into_iter().map(|(_, u, _)| u).collect::<Vec<_>>();
            for fs_uuid in fs_uuids {
                match register_filesystem(engine, connection, manager, counter, pool_uuid, fs_uuid).await {
                    Ok(op) => fs_paths.push(op),
                    Err(_) => {
                        warn!("Unable to register object path for filesystem with UUID {fs_uuid} belonging to pool {pool_uuid} on the D-Bus");
                    },
                }
            }

            let mut bd_paths = Vec::new();
            let bd_uuids = pool.blockdevs().into_iter().map(|(u, _, _)| u).collect::<Vec<_>>();
            for dev_uuid in bd_uuids {
                match register_blockdev(engine, connection, manager, counter, pool_uuid, dev_uuid).await {
                    Ok(op) => bd_paths.push(op),
                    Err(_) => {
                        warn!("Unable to register object path for blockdev with UUID {dev_uuid} belonging to pool {pool_uuid} on the D-Bus");
                    },
                }
            }

            Ok((path, fs_paths, bd_paths))
        }
        None => {
            Err(StratisError::Msg(format!("Pool with {pool_uuid} was successfully started but appears to have been removed before it could be exposed on the D-Bus")))
        }
    }
}

pub async fn unregister_pool(
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    path: &ObjectPath<'_>,
    fs_uuids: &[FilesystemUuid],
    dev_uuids: &[DevUuid],
) -> StratisResult<PoolUuid> {
    // Unregister all filesystems
    for fs_uuid in fs_uuids {
        let maybe_fs_path = manager.write().await.filesystem_get_path(fs_uuid).cloned();
        if let Some(fs_path) = maybe_fs_path {
            if let Err(e) = unregister_filesystem(connection, manager, &fs_path).await {
                warn!("Failed to unregister {fs_path} representing filesystem {fs_uuid}: {e}");
            }
        }
    }

    // Unregister all blockdevs
    for dev_uuid in dev_uuids {
        let maybe_dev_path = manager.write().await.blockdev_get_path(dev_uuid).cloned();
        if let Some(dev_path) = maybe_dev_path {
            if let Err(e) = unregister_blockdev(connection, manager, &dev_path).await {
                warn!("Failed to unregister {dev_path} representing blockdev {dev_uuid}: {e}");
            }
        }
    }

    // Unregister the pool itself
    let uuid = {
        let mut lock = manager.write().await;
        let uuid = lock
            .pool_get_uuid(path)
            .ok_or_else(|| StratisError::Msg(format!("No UUID associated with path {path}")))?;
        lock.remove_pool(path);
        uuid
    };

    if let Err(e) = PoolInterface::unregister(connection, path.clone()).await {
        warn!("Failed to deregister interface pool for path {path}: {e}");
    }

    Ok(uuid)
}

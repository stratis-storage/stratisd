// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::HashMap,
    sync::{atomic::AtomicU64, Arc},
};

use tokio::sync::RwLock;
use zbus::{
    fdo::ObjectManager,
    zvariant::{ObjectPath, OwnedObjectPath},
    Connection,
};

use crate::{
    dbus::consts::STRATIS_BASE_PATH,
    engine::{DevUuid, Engine, FilesystemUuid, Lockable, PoolUuid},
    stratis::StratisResult,
};

mod manager_3_0;
mod manager_3_8;
mod manager_3_9;

pub use manager_3_0::ManagerR0;
pub use manager_3_9::ManagerR9;

#[derive(Default)]
pub struct Manager {
    pool_path_to_uuid: HashMap<OwnedObjectPath, PoolUuid>,
    pool_uuid_to_path: HashMap<PoolUuid, OwnedObjectPath>,
    filesystem_path_to_uuid: HashMap<OwnedObjectPath, FilesystemUuid>,
    filesystem_uuid_to_path: HashMap<FilesystemUuid, OwnedObjectPath>,
    blockdev_path_to_uuid: HashMap<OwnedObjectPath, DevUuid>,
    blockdev_uuid_to_path: HashMap<DevUuid, OwnedObjectPath>,
}

impl Manager {
    pub fn add_pool(&mut self, path: &ObjectPath<'_>, uuid: PoolUuid) {
        self.pool_path_to_uuid
            .insert(OwnedObjectPath::from(path.clone()), uuid);
        self.pool_uuid_to_path
            .insert(uuid, OwnedObjectPath::from(path.clone()));
    }

    pub fn add_filesystem(&mut self, path: &ObjectPath<'_>, uuid: FilesystemUuid) {
        self.filesystem_path_to_uuid
            .insert(OwnedObjectPath::from(path.clone()), uuid);
        self.filesystem_uuid_to_path
            .insert(uuid, OwnedObjectPath::from(path.clone()));
    }

    pub fn add_blockdev(&mut self, path: &ObjectPath<'_>, uuid: DevUuid) {
        self.blockdev_path_to_uuid
            .insert(OwnedObjectPath::from(path.clone()), uuid);
        self.blockdev_uuid_to_path
            .insert(uuid, OwnedObjectPath::from(path.clone()));
    }

    pub fn pool_get_uuid(&self, path: &ObjectPath<'_>) -> Option<PoolUuid> {
        self.pool_path_to_uuid.get(path).cloned()
    }

    pub fn pool_get_path(&self, uuid: &PoolUuid) -> Option<&OwnedObjectPath> {
        self.pool_uuid_to_path.get(uuid)
    }

    pub fn filesystem_get_uuid(&self, path: &ObjectPath<'_>) -> Option<FilesystemUuid> {
        self.filesystem_path_to_uuid.get(path).cloned()
    }

    pub fn filesystem_get_path(&self, uuid: &FilesystemUuid) -> Option<&OwnedObjectPath> {
        self.filesystem_uuid_to_path.get(uuid)
    }

    pub fn blockdev_get_uuid(&self, path: &ObjectPath<'_>) -> Option<DevUuid> {
        self.blockdev_path_to_uuid.get(path).cloned()
    }

    pub fn blockdev_get_path(&self, uuid: &DevUuid) -> Option<&OwnedObjectPath> {
        self.blockdev_uuid_to_path.get(uuid)
    }

    pub fn remove_pool(&mut self, path: &ObjectPath<'_>) {
        let uuid = self.pool_path_to_uuid.remove(path);
        if let Some(ref u) = uuid {
            self.pool_uuid_to_path.remove(u);
        }
    }

    pub fn remove_filesystem(&mut self, path: &ObjectPath<'_>) {
        let uuid = self.filesystem_path_to_uuid.remove(path);
        if let Some(ref u) = uuid {
            self.filesystem_uuid_to_path.remove(u);
        }
    }

    pub fn remove_blockdev(&mut self, path: &ObjectPath<'_>) {
        let uuid = self.blockdev_path_to_uuid.remove(path);
        if let Some(ref u) = uuid {
            self.blockdev_uuid_to_path.remove(u);
        }
    }
}

pub async fn register_manager(
    connection: &Arc<Connection>,
    engine: Arc<dyn Engine>,
    manager: Lockable<Arc<RwLock<Manager>>>,
    counter: Arc<AtomicU64>,
) -> StratisResult<()> {
    ManagerR0::register(
        connection,
        Arc::clone(&engine),
        manager.clone(),
        Arc::clone(&counter),
    )
    .await?;
    ManagerR9::register(connection, engine, manager, counter).await?;
    connection
        .object_server()
        .at(STRATIS_BASE_PATH, ObjectManager)
        .await?;
    Ok(())
}

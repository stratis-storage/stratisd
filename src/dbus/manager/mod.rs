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
    stratis::{StratisError, StratisResult},
};

mod manager_3_0;
mod manager_3_1;
mod manager_3_2;
mod manager_3_3;
mod manager_3_4;
mod manager_3_5;
mod manager_3_6;
mod manager_3_7;
mod manager_3_8;
mod manager_3_9;
mod report_3_0;
mod report_3_1;
mod report_3_2;
mod report_3_3;
mod report_3_4;
mod report_3_5;
mod report_3_6;
mod report_3_7;
mod report_3_8;
mod report_3_9;

pub use manager_3_0::ManagerR0;
pub use manager_3_1::ManagerR1;
pub use manager_3_2::ManagerR2;
pub use manager_3_3::ManagerR3;
pub use manager_3_4::ManagerR4;
pub use manager_3_5::ManagerR5;
pub use manager_3_6::ManagerR6;
pub use manager_3_7::ManagerR7;
pub use manager_3_8::ManagerR8;
pub use manager_3_9::ManagerR9;
pub use report_3_0::ReportR0;
pub use report_3_1::ReportR1;
pub use report_3_2::ReportR2;
pub use report_3_3::ReportR3;
pub use report_3_4::ReportR4;
pub use report_3_5::ReportR5;
pub use report_3_6::ReportR6;
pub use report_3_7::ReportR7;
pub use report_3_8::ReportR8;
pub use report_3_9::ReportR9;

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
    pub fn add_pool(&mut self, path: &ObjectPath<'_>, uuid: PoolUuid) -> StratisResult<()> {
        match (
            self.pool_path_to_uuid.get(path),
            self.pool_uuid_to_path.get(&uuid),
        ) {
            (Some(u), Some(p)) => {
                if uuid == *u && path == &p.as_ref() {
                    Ok(())
                } else {
                    Err(StratisError::Msg(format!("Attempted to add path {path}, UUID {uuid} for pool but path {p}, UUID {u} pair already exists")))
                }
            }
            (Some(u), _) => Err(StratisError::Msg(format!(
                "Attempted to add path {path}, UUID {uuid} for pool but path {path}, UUID {u} pair already exists"
            ))),
            (_, Some(p)) => Err(StratisError::Msg(format!(
                "Attempted to add path {path}, UUID {uuid} for pool but path {p}, UUID {uuid} pair already exists"
            ))),
            (None, None) => {
                self.pool_path_to_uuid
                    .insert(OwnedObjectPath::from(path.clone()), uuid);
                self.pool_uuid_to_path
                    .insert(uuid, OwnedObjectPath::from(path.clone()));
                Ok(())
            }
        }
    }

    pub fn add_filesystem(
        &mut self,
        path: &ObjectPath<'_>,
        uuid: FilesystemUuid,
    ) -> StratisResult<()> {
        match (
            self.filesystem_path_to_uuid.get(path),
            self.filesystem_uuid_to_path.get(&uuid),
        ) {
            (Some(u), Some(p)) => {
                if uuid == *u && path == &p.as_ref() {
                    Ok(())
                } else {
                    Err(StratisError::Msg(format!("Attempted to add path {path}, UUID {uuid} for filesystem but path {p}, UUID {u} pair already exists")))
                }
            }
            (Some(u), _) => Err(StratisError::Msg(format!(
                "Attempted to add path {path}, UUID {uuid} for filesystem but path {path}, UUID {u} pair already exists"
            ))),
            (_, Some(p)) => Err(StratisError::Msg(format!(
                "Attempted to add path {path}, UUID {uuid} for filesystem  but path {p}, UUID {uuid} pair already exists"
            ))),
            (None, None) => {
                self.filesystem_path_to_uuid
                    .insert(OwnedObjectPath::from(path.clone()), uuid);
                self.filesystem_uuid_to_path
                    .insert(uuid, OwnedObjectPath::from(path.clone()));
                Ok(())
            }
        }
    }

    pub fn add_blockdev(&mut self, path: &ObjectPath<'_>, uuid: DevUuid) -> StratisResult<()> {
        match (
            self.blockdev_path_to_uuid.get(path),
            self.blockdev_uuid_to_path.get(&uuid),
        ) {
            (Some(u), Some(p)) => {
                if uuid == *u && path == &p.as_ref() {
                    Ok(())
                } else {
                    Err(StratisError::Msg(format!("Attempted to add path {path}, UUID {uuid} for blockdev but path {p}, UUID {u} pair already exists")))
                }
            }
            (Some(u), _) => Err(StratisError::Msg(format!(
                "Attempted to add path {path}, UUID {uuid} for blockdev but path {path}, UUID {u} pair already exists"
            ))),
            (_, Some(p)) => Err(StratisError::Msg(format!(
                "Attempted to add path {path}, UUID {uuid} for blockdev but path {p}, UUID {uuid} pair already exists"
            ))),
            (None, None) => {
                self.blockdev_path_to_uuid
                    .insert(OwnedObjectPath::from(path.clone()), uuid);
                self.blockdev_uuid_to_path
                    .insert(uuid, OwnedObjectPath::from(path.clone()));
                Ok(())
            }
        }
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
    engine: &Arc<dyn Engine>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
) {
    if let Err(e) = ManagerR0::register(engine, connection, manager, counter).await {
        warn!("Failed to register interface Manager.r0: {e}");
    }
    if let Err(e) = ManagerR1::register(engine, connection, manager, counter).await {
        warn!("Failed to register interface Manager.r1: {e}");
    }
    if let Err(e) = ManagerR2::register(engine, connection, manager, counter).await {
        warn!("Failed to register interface Manager.r2: {e}");
    }
    if let Err(e) = ManagerR3::register(engine, connection, manager, counter).await {
        warn!("Failed to register interface Manager.r3: {e}");
    }
    if let Err(e) = ManagerR4::register(engine, connection, manager, counter).await {
        warn!("Failed to register interface Manager.r4: {e}");
    }
    if let Err(e) = ManagerR5::register(engine, connection, manager, counter).await {
        warn!("Failed to register interface Manager.r5: {e}");
    }
    if let Err(e) = ManagerR6::register(engine, connection, manager, counter).await {
        warn!("Failed to register interface Manager.r6: {e}");
    }
    if let Err(e) = ManagerR7::register(engine, connection, manager, counter).await {
        warn!("Failed to register interface Manager.r7: {e}");
    }
    if let Err(e) = ManagerR8::register(engine, connection, manager, counter).await {
        warn!("Failed to register interface Manager.r8: {e}");
    }
    if let Err(e) = ManagerR9::register(engine, connection, manager, counter).await {
        warn!("Failed to register interface Manager.r9: {e}");
    }
    if let Err(e) = ReportR0::register(engine, connection).await {
        warn!("Failed to register interface Report.r0: {e}");
    }
    if let Err(e) = ReportR1::register(engine, connection).await {
        warn!("Failed to register interface Report.r1: {e}");
    }
    if let Err(e) = ReportR2::register(engine, connection).await {
        warn!("Failed to register interface Report.r2: {e}");
    }
    if let Err(e) = ReportR3::register(engine, connection).await {
        warn!("Failed to register interface Report.r3: {e}");
    }
    if let Err(e) = ReportR4::register(engine, connection).await {
        warn!("Failed to register interface Report.r4: {e}");
    }
    if let Err(e) = ReportR5::register(engine, connection).await {
        warn!("Failed to register interface Report.r5: {e}");
    }
    if let Err(e) = ReportR6::register(engine, connection).await {
        warn!("Failed to register interface Report.r6: {e}");
    }
    if let Err(e) = ReportR7::register(engine, connection).await {
        warn!("Failed to register interface Report.r7: {e}");
    }
    if let Err(e) = ReportR8::register(engine, connection).await {
        warn!("Failed to register interface Report.r8: {e}");
    }
    if let Err(e) = ReportR9::register(engine, connection).await {
        warn!("Failed to register interface Report.r9: {e}");
    }
    if let Err(e) = connection
        .object_server()
        .at(STRATIS_BASE_PATH, ObjectManager)
        .await
    {
        warn!("Failed to register ObjectManager at {STRATIS_BASE_PATH}: {e}");
    }
}

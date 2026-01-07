// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use tokio::sync::RwLock;
use zbus::{
    fdo::Error,
    interface,
    zvariant::{ObjectPath, OwnedObjectPath},
    Connection,
};

use crate::{
    dbus::{
        filesystem::{
            filesystem_3_0::{
                created_prop, devnode_prop, name_prop, pool_prop, set_name_method, size_prop,
                used_prop,
            },
            filesystem_3_6::{set_size_limit_prop, size_limit_prop},
            filesystem_3_7::{merge_scheduled_prop, origin_prop, set_merge_scheduled_prop},
            shared::{filesystem_prop, set_filesystem_prop},
        },
        manager::Manager,
    },
    engine::{Engine, FilesystemUuid, Lockable, Name, PoolUuid},
    stratis::StratisResult,
};

pub struct FilesystemR9 {
    engine: Arc<dyn Engine>,
    connection: Arc<Connection>,
    manager: Lockable<Arc<RwLock<Manager>>>,
    parent_uuid: PoolUuid,
    uuid: FilesystemUuid,
}

impl FilesystemR9 {
    fn new(
        engine: Arc<dyn Engine>,
        connection: Arc<Connection>,
        manager: Lockable<Arc<RwLock<Manager>>>,
        parent_uuid: PoolUuid,
        uuid: FilesystemUuid,
    ) -> Self {
        FilesystemR9 {
            engine,
            connection,
            manager,
            parent_uuid,
            uuid,
        }
    }

    pub async fn register(
        engine: Arc<dyn Engine>,
        connection: &Arc<Connection>,
        manager: &Lockable<Arc<RwLock<Manager>>>,
        path: ObjectPath<'_>,
        parent_uuid: PoolUuid,
        uuid: FilesystemUuid,
    ) -> StratisResult<()> {
        let filesystem = Self::new(
            engine,
            Arc::clone(connection),
            manager.clone(),
            parent_uuid,
            uuid,
        );

        connection.object_server().at(path, filesystem).await?;
        Ok(())
    }

    pub async fn unregister(
        connection: &Arc<Connection>,
        path: ObjectPath<'_>,
    ) -> StratisResult<()> {
        connection
            .object_server()
            .remove::<FilesystemR9, _>(path)
            .await?;
        Ok(())
    }
}

#[interface(
    name = "org.storage.stratis3.filesystem.r9",
    introspection_docs = false
)]
impl FilesystemR9 {
    #[zbus(out_args("result", "return_code", "return_string"))]
    async fn set_name(&self, name: &str) -> ((bool, String), u16, String) {
        set_name_method(
            &self.engine,
            &self.connection,
            &self.manager,
            self.parent_uuid,
            self.uuid,
            name,
        )
        .await
    }

    #[zbus(property(emits_changed_signal = "const"))]
    async fn created(&self) -> Result<String, Error> {
        filesystem_prop(&self.engine, self.parent_uuid, self.uuid, created_prop).await
    }

    #[zbus(property(emits_changed_signal = "invalidates"))]
    async fn devnode(&self) -> Result<String, Error> {
        filesystem_prop(&self.engine, self.parent_uuid, self.uuid, devnode_prop).await
    }

    #[zbus(property)]
    async fn merge_scheduled(&self) -> Result<bool, Error> {
        filesystem_prop(
            &self.engine,
            self.parent_uuid,
            self.uuid,
            merge_scheduled_prop,
        )
        .await
    }

    #[zbus(property)]
    async fn set_merge_scheduled(&self, value: bool) -> Result<(), zbus::Error> {
        set_filesystem_prop(
            &self.engine,
            self.parent_uuid,
            self.uuid,
            value,
            set_merge_scheduled_prop,
        )
        .await
    }

    #[zbus(property)]
    async fn name(&self) -> Result<Name, Error> {
        filesystem_prop(&self.engine, self.parent_uuid, self.uuid, name_prop).await
    }

    #[zbus(property)]
    async fn origin(&self) -> Result<(bool, FilesystemUuid), Error> {
        filesystem_prop(&self.engine, self.parent_uuid, self.uuid, origin_prop).await
    }

    #[zbus(property(emits_changed_signal = "const"))]
    async fn pool(&self) -> Result<OwnedObjectPath, Error> {
        pool_prop(self.manager.read().await, self.parent_uuid)
    }

    #[zbus(property)]
    async fn size(&self) -> Result<String, Error> {
        filesystem_prop(&self.engine, self.parent_uuid, self.uuid, size_prop).await
    }

    #[zbus(property)]
    async fn size_limit(&self) -> Result<(bool, String), Error> {
        filesystem_prop(&self.engine, self.parent_uuid, self.uuid, size_limit_prop).await
    }

    #[zbus(property)]
    async fn set_size_limit(&self, value: (bool, String)) -> Result<(), zbus::Error> {
        set_filesystem_prop(
            &self.engine,
            self.parent_uuid,
            self.uuid,
            value,
            set_size_limit_prop,
        )
        .await
    }

    #[zbus(property)]
    async fn used(&self) -> Result<(bool, String), Error> {
        filesystem_prop(&self.engine, self.parent_uuid, self.uuid, used_prop).await
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn uuid(&self) -> FilesystemUuid {
        self.uuid
    }
}

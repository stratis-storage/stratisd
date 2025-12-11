// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use tokio::sync::RwLock;
use zbus::{
    self,
    fdo::Error,
    interface,
    zvariant::{ObjectPath, OwnedObjectPath},
    Connection,
};

use crate::{
    dbus::{
        blockdev::shared::{blockdev_prop, set_blockdev_prop},
        manager::Manager,
    },
    engine::{DevUuid, Engine, Lockable, PoolUuid},
    stratis::StratisResult,
};

use crate::dbus::blockdev::blockdev_3_0::{
    devnode_prop, hardware_info_prop, init_time_prop, physical_path_prop, pool_prop, tier_prop,
    total_physical_size_prop, user_info_prop,
};

use crate::dbus::blockdev::blockdev_3_3::{new_physical_size_prop, set_user_info_prop};

pub struct BlockdevR6 {
    engine: Arc<dyn Engine>,
    manager: Lockable<Arc<RwLock<Manager>>>,
    parent_uuid: PoolUuid,
    uuid: DevUuid,
}

impl BlockdevR6 {
    fn new(
        engine: Arc<dyn Engine>,
        manager: Lockable<Arc<RwLock<Manager>>>,
        parent_uuid: PoolUuid,
        uuid: DevUuid,
    ) -> Self {
        BlockdevR6 {
            engine,
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
        uuid: DevUuid,
    ) -> StratisResult<()> {
        let blockdev = Self::new(engine, manager.clone(), parent_uuid, uuid);

        connection.object_server().at(path, blockdev).await?;
        Ok(())
    }

    pub async fn unregister(
        connection: &Arc<Connection>,
        path: ObjectPath<'_>,
    ) -> StratisResult<()> {
        connection
            .object_server()
            .remove::<BlockdevR6, _>(path)
            .await?;
        Ok(())
    }
}

#[interface(name = "org.storage.stratis3.blockdev.r6", introspection_docs = false)]
impl BlockdevR6 {
    #[zbus(property(emits_changed_signal = "const"))]
    #[allow(non_snake_case)]
    async fn devnode(&self) -> Result<String, Error> {
        blockdev_prop(&self.engine, self.parent_uuid, self.uuid, devnode_prop).await
    }

    #[zbus(property(emits_changed_signal = "const"))]
    async fn hardware_info(&self) -> Result<(bool, String), Error> {
        blockdev_prop(
            &self.engine,
            self.parent_uuid,
            self.uuid,
            hardware_info_prop,
        )
        .await
    }

    #[zbus(property)]
    async fn user_info(&self) -> Result<(bool, String), Error> {
        blockdev_prop(&self.engine, self.parent_uuid, self.uuid, user_info_prop).await
    }

    #[zbus(property)]
    async fn set_user_info(&self, value: (bool, String)) -> Result<(), zbus::Error> {
        set_blockdev_prop(
            &self.engine,
            self.parent_uuid,
            self.uuid,
            value,
            set_user_info_prop,
        )
        .await
    }

    #[zbus(property(emits_changed_signal = "const"))]
    async fn initialization_time(&self) -> Result<u64, Error> {
        blockdev_prop(&self.engine, self.parent_uuid, self.uuid, init_time_prop)
            .await
            .and_then(|r| r)
    }

    #[zbus(property(emits_changed_signal = "const"))]
    async fn pool(&self) -> Result<OwnedObjectPath, Error> {
        pool_prop(self.manager.read().await, self.parent_uuid)
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn uuid(&self) -> DevUuid {
        self.uuid
    }

    #[zbus(property(emits_changed_signal = "false"))]
    async fn tier(&self) -> Result<u16, Error> {
        blockdev_prop(&self.engine, self.parent_uuid, self.uuid, tier_prop).await
    }

    #[zbus(property(emits_changed_signal = "const"))]
    async fn physical_path(&self) -> Result<String, Error> {
        blockdev_prop(
            &self.engine,
            self.parent_uuid,
            self.uuid,
            physical_path_prop,
        )
        .await
    }

    #[zbus(property)]
    async fn total_physical_size(&self) -> Result<String, Error> {
        blockdev_prop(
            &self.engine,
            self.parent_uuid,
            self.uuid,
            total_physical_size_prop,
        )
        .await
    }

    #[zbus(property)]
    async fn new_physical_size(&self) -> Result<(bool, String), Error> {
        blockdev_prop(
            &self.engine,
            self.parent_uuid,
            self.uuid,
            new_physical_size_prop,
        )
        .await
    }
}

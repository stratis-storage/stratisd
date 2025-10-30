// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{atomic::AtomicU64, Arc};

use tokio::sync::RwLock;
use zbus::{fdo::Error, interface, zvariant::ObjectPath, Connection};

use crate::{
    dbus::{
        manager::Manager,
        pool::{
            pool_3_0::{allocated_prop, name_prop, size_prop, used_prop},
            pool_3_6::create_filesystems_method,
            shared::{pool_prop, try_pool_prop},
        },
        types::FilesystemSpec,
    },
    engine::{self, Engine, Lockable, PoolUuid},
    stratis::StratisResult,
};

pub struct PoolR9 {
    connection: Arc<Connection>,
    engine: Arc<dyn Engine>,
    manager: Lockable<Arc<RwLock<Manager>>>,
    counter: Arc<AtomicU64>,
    uuid: PoolUuid,
}

impl PoolR9 {
    fn new(
        engine: Arc<dyn Engine>,
        connection: Arc<Connection>,
        manager: Lockable<Arc<RwLock<Manager>>>,
        counter: Arc<AtomicU64>,
        uuid: PoolUuid,
    ) -> Self {
        PoolR9 {
            connection,
            engine,
            manager,
            counter,
            uuid,
        }
    }

    pub async fn register(
        engine: &Arc<dyn Engine>,
        connection: &Arc<Connection>,
        manager: &Lockable<Arc<RwLock<Manager>>>,
        counter: &Arc<AtomicU64>,
        path: ObjectPath<'_>,
        uuid: PoolUuid,
    ) -> StratisResult<()> {
        let pool = Self::new(
            Arc::clone(engine),
            Arc::clone(connection),
            manager.clone(),
            Arc::clone(counter),
            uuid,
        );

        connection.object_server().at(path, pool).await?;
        Ok(())
    }
}

#[interface(name = "org.storage.stratis3.pool.r9")]
impl PoolR9 {
    #[allow(non_snake_case)]
    async fn CreateFilesystems(
        &self,
        specs: FilesystemSpec<'_>,
    ) -> ((bool, Vec<ObjectPath<'_>>), u16, String) {
        create_filesystems_method(
            &self.engine,
            &self.connection,
            &self.manager,
            &self.counter,
            self.uuid,
            specs,
        )
        .await
    }

    #[zbus(property(emits_changed_signal = "const"))]
    #[allow(non_snake_case)]
    fn Uuid(&self) -> PoolUuid {
        self.uuid
    }

    #[zbus(property(emits_changed_signal = "const"))]
    #[allow(non_snake_case)]
    async fn Name(&self) -> Result<engine::Name, Error> {
        pool_prop(&self.engine, self.uuid, name_prop).await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    #[allow(non_snake_case)]
    async fn TotalPhysicalSize(&self) -> Result<String, Error> {
        pool_prop(&self.engine, self.uuid, size_prop).await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    #[allow(non_snake_case)]
    async fn TotalPhysicalUsed(&self) -> Result<(bool, String), Error> {
        try_pool_prop(&self.engine, self.uuid, used_prop).await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    #[allow(non_snake_case)]
    async fn AllocatedSize(&self) -> Result<String, Error> {
        pool_prop(&self.engine, self.uuid, allocated_prop).await
    }
}

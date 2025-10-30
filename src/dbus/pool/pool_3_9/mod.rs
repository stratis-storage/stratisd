// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use zbus::{
    fdo::Error,
    interface,
    zvariant::{ObjectPath, OwnedValue},
    Connection,
};

use crate::{
    dbus::pool::{
        pool_3_0::{allocated_prop, name_prop, size_prop, used_prop, uuid_prop},
        shared::{pool_prop, try_pool_prop},
    },
    engine::{Engine, PoolUuid},
    stratis::StratisResult,
};

pub struct PoolR9 {
    _connection: Arc<Connection>,
    engine: Arc<dyn Engine>,
    uuid: PoolUuid,
}

impl PoolR9 {
    fn new(connection: Arc<Connection>, engine: Arc<dyn Engine>, uuid: PoolUuid) -> Self {
        PoolR9 {
            _connection: connection,
            engine,
            uuid,
        }
    }

    pub async fn register(
        connection: &Arc<Connection>,
        path: ObjectPath<'_>,
        engine: Arc<dyn Engine>,
        uuid: PoolUuid,
    ) -> StratisResult<()> {
        let pool = Self::new(Arc::clone(connection), engine, uuid);

        connection.object_server().at(path, pool).await?;
        Ok(())
    }
}

#[interface(name = "org.storage.stratis3.pool.r9")]
impl PoolR9 {
    #[zbus(property(emits_changed_signal = "const"))]
    #[allow(non_snake_case)]
    fn Uuid(&self) -> String {
        uuid_prop(self.uuid)
    }

    #[zbus(property(emits_changed_signal = "const"))]
    #[allow(non_snake_case)]
    async fn Name(&self) -> Result<OwnedValue, Error> {
        pool_prop(&self.engine, self.uuid, name_prop).await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    #[allow(non_snake_case)]
    async fn TotalPhysicalSize(&self) -> Result<OwnedValue, Error> {
        pool_prop(&self.engine, self.uuid, size_prop).await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    #[allow(non_snake_case)]
    async fn TotalPhysicalUsed(&self) -> Result<OwnedValue, Error> {
        try_pool_prop(&self.engine, self.uuid, used_prop).await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    #[allow(non_snake_case)]
    async fn AllocatedSize(&self) -> Result<OwnedValue, Error> {
        pool_prop(&self.engine, self.uuid, allocated_prop).await
    }
}

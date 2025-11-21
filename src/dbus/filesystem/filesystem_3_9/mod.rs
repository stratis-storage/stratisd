// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use zbus::{fdo::Error, interface, zvariant::ObjectPath, Connection};

use crate::{
    dbus::filesystem::{filesystem_3_0::name_prop, shared::filesystem_prop},
    engine::{self, Engine, FilesystemUuid, PoolUuid},
    stratis::StratisResult,
};

pub struct FilesystemR9 {
    engine: Arc<dyn Engine>,
    parent_uuid: PoolUuid,
    uuid: FilesystemUuid,
}

impl FilesystemR9 {
    fn new(engine: Arc<dyn Engine>, parent_uuid: PoolUuid, uuid: FilesystemUuid) -> Self {
        FilesystemR9 {
            engine,
            parent_uuid,
            uuid,
        }
    }

    pub async fn register(
        engine: &Arc<dyn Engine>,
        connection: &Arc<Connection>,
        path: ObjectPath<'_>,
        parent_uuid: PoolUuid,
        uuid: FilesystemUuid,
    ) -> StratisResult<()> {
        let filesystem = Self::new(Arc::clone(engine), parent_uuid, uuid);

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

#[interface(name = "org.storage.stratis3.filesystem.r9")]
impl FilesystemR9 {
    #[zbus(property(emits_changed_signal = "const"))]
    #[allow(non_snake_case)]
    fn Uuid(&self) -> FilesystemUuid {
        self.uuid
    }

    #[zbus(property(emits_changed_signal = "const"))]
    #[allow(non_snake_case)]
    async fn Name(&self) -> Result<engine::Name, Error> {
        filesystem_prop(&self.engine, self.parent_uuid, self.uuid, name_prop).await
    }
}

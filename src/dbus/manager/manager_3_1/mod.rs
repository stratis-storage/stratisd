// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc},
};

use tokio::sync::RwLock;
use zbus::{
    interface,
    zvariant::{Fd, ObjectPath},
    Connection, Result,
};

use crate::{
    dbus::{
        consts,
        manager::manager_3_0::{
            create_pool_method, destroy_pool_method, list_keys_method, locked_pools_prop,
            set_key_method, unlock_pool_method, unset_key_method, version_prop,
        },
        manager::Manager,
    },
    engine::{DevUuid, Engine, KeyDescription, Lockable, LockedPoolsInfo, PoolUuid, UnlockMethod},
};

pub struct ManagerR1 {
    connection: Arc<Connection>,
    engine: Arc<dyn Engine>,
    manager: Lockable<Arc<RwLock<Manager>>>,
    counter: Arc<AtomicU64>,
}

impl ManagerR1 {
    pub fn new(
        engine: Arc<dyn Engine>,
        connection: Arc<Connection>,
        manager: Lockable<Arc<RwLock<Manager>>>,
        counter: Arc<AtomicU64>,
    ) -> Self {
        ManagerR1 {
            connection,
            engine,
            manager,
            counter,
        }
    }

    pub async fn register(
        engine: &Arc<dyn Engine>,
        connection: &Arc<Connection>,
        manager: &Lockable<Arc<RwLock<Manager>>>,
        counter: &Arc<AtomicU64>,
    ) -> Result<()> {
        let manager = Self::new(
            Arc::clone(engine),
            Arc::clone(connection),
            manager.clone(),
            Arc::clone(counter),
        );
        connection
            .object_server()
            .at(consts::STRATIS_BASE_PATH, manager)
            .await?;
        Ok(())
    }
}

#[interface(name = "org.storage.stratis3.Manager.r1")]
impl ManagerR1 {
    #[zbus(property(emits_changed_signal = "const"))]
    #[allow(non_snake_case)]
    #[allow(clippy::unused_self)]
    fn Version(&self) -> &str {
        version_prop()
    }

    #[zbus(property(emits_changed_signal = "true"))]
    #[allow(non_snake_case)]
    async fn LockedPools(&self) -> LockedPoolsInfo {
        locked_pools_prop(&self.engine).await
    }

    #[allow(non_snake_case)]
    async fn ListKeys(&self) -> (Vec<KeyDescription>, u16, String) {
        list_keys_method(&self.engine).await
    }

    #[allow(non_snake_case)]
    async fn SetKey(
        &self,
        key_desc: KeyDescription,
        key_fd: Fd<'_>,
    ) -> ((bool, bool), u16, String) {
        set_key_method(&self.engine, &key_desc, key_fd).await
    }

    #[allow(non_snake_case)]
    async fn UnsetKey(&self, key_desc: KeyDescription) -> (bool, u16, String) {
        unset_key_method(&self.engine, &key_desc).await
    }

    #[allow(non_snake_case)]
    #[allow(clippy::too_many_arguments)]
    async fn CreatePool(
        &self,
        name: &str,
        #[allow(unused_variables)] redundancy: (bool, u16),
        devs: Vec<PathBuf>,
        key_desc: (bool, KeyDescription),
        clevis_info: (bool, (&str, &str)),
    ) -> ((bool, (ObjectPath<'_>, Vec<ObjectPath<'_>>)), u16, String) {
        create_pool_method(
            &self.engine,
            &self.connection,
            &self.manager,
            &self.counter,
            name,
            devs,
            key_desc,
            clevis_info,
        )
        .await
    }

    #[allow(non_snake_case)]
    async fn DestroyPool(&self, pool: ObjectPath<'_>) -> ((bool, String), u16, String) {
        destroy_pool_method(&self.engine, &self.connection, &self.manager, pool).await
    }

    #[allow(non_snake_case)]
    async fn UnlockPool(
        &self,
        pool_uuid: PoolUuid,
        unlock_method: UnlockMethod,
    ) -> ((bool, Vec<DevUuid>), u16, String) {
        unlock_pool_method(&self.engine, &self.connection, pool_uuid, unlock_method).await
    }
}

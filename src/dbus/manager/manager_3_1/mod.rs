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
    zvariant::{Fd, ObjectPath, OwnedObjectPath},
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
    engine::{Engine, KeyDescription, Lockable, LockedPoolsInfo, UnlockMethod},
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
    #[allow(clippy::unused_self)]
    fn version(&self) -> &str {
        version_prop()
    }

    #[zbus(property(emits_changed_signal = "true"))]
    async fn locked_pools(&self) -> LockedPoolsInfo {
        locked_pools_prop(&self.engine).await
    }

    async fn list_keys(&self) -> (Vec<KeyDescription>, u16, String) {
        list_keys_method(&self.engine).await
    }

    async fn set_key(
        &self,
        key_desc: KeyDescription,
        key_fd: Fd<'_>,
    ) -> ((bool, bool), u16, String) {
        set_key_method(&self.engine, &key_desc, key_fd).await
    }

    async fn unset_key(&self, key_desc: KeyDescription) -> (bool, u16, String) {
        unset_key_method(&self.engine, &key_desc).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_pool(
        &self,
        name: &str,
        #[allow(unused_variables)] redundancy: (bool, u16),
        devs: Vec<PathBuf>,
        key_desc: (bool, KeyDescription),
        clevis_info: (bool, (&str, &str)),
    ) -> ((bool, (OwnedObjectPath, Vec<OwnedObjectPath>)), u16, String) {
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

    async fn destroy_pool(&self, pool: ObjectPath<'_>) -> ((bool, String), u16, String) {
        destroy_pool_method(&self.engine, &self.connection, &self.manager, pool).await
    }

    async fn unlock_pool(
        &self,
        pool_uuid: &str,
        unlock_method: UnlockMethod,
    ) -> ((bool, Vec<String>), u16, String) {
        unlock_pool_method(&self.engine, &self.connection, pool_uuid, unlock_method).await
    }
}

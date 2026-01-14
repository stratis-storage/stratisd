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
            create_pool_method, destroy_pool_method, engine_state_report_method, list_keys_method,
            set_key_method, unset_key_method, version_prop,
        },
        manager::Manager,
        types,
    },
    engine::{Engine, KeyDescription, Lockable, PoolUuid, StoppedPoolsInfo, UnlockMethod},
};

mod methods;
mod props;

pub use methods::{refresh_state_method, start_pool_method, stop_pool_method};
pub use props::stopped_pools_prop;

pub struct ManagerR2 {
    connection: Arc<Connection>,
    engine: Arc<dyn Engine>,
    manager: Lockable<Arc<RwLock<Manager>>>,
    counter: Arc<AtomicU64>,
}

impl ManagerR2 {
    pub fn new(
        engine: Arc<dyn Engine>,
        connection: Arc<Connection>,
        manager: Lockable<Arc<RwLock<Manager>>>,
        counter: Arc<AtomicU64>,
    ) -> Self {
        ManagerR2 {
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

#[interface(name = "org.storage.stratis3.Manager.r2")]
impl ManagerR2 {
    #[zbus(property(emits_changed_signal = "const"))]
    #[allow(clippy::unused_self)]
    fn version(&self) -> &str {
        version_prop()
    }

    #[zbus(property(emits_changed_signal = "true"))]
    async fn stopped_pools(&self) -> types::ManagerR2<StoppedPoolsInfo> {
        stopped_pools_prop(&self.engine).await
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

    async fn start_pool(
        &self,
        pool_uuid: PoolUuid,
        unlock_method: (bool, UnlockMethod),
    ) -> (
        (
            bool,
            (OwnedObjectPath, Vec<OwnedObjectPath>, Vec<OwnedObjectPath>),
        ),
        u16,
        String,
    ) {
        start_pool_method(
            &self.engine,
            &self.connection,
            &self.manager,
            &self.counter,
            pool_uuid,
            unlock_method,
        )
        .await
    }

    async fn stop_pool(&self, pool: ObjectPath<'_>) -> ((bool, String), u16, String) {
        stop_pool_method(&self.engine, &self.connection, &self.manager, pool).await
    }

    async fn refresh_state(&self) -> (u16, String) {
        refresh_state_method(&self.engine).await
    }

    #[allow(non_snake_case)]
    fn EngineStateReport(&self) -> (String, u16, String) {
        engine_state_report_method(&self.engine)
    }
}

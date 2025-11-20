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
        manager::Manager,
        manager::{
            manager_3_0::{
                destroy_pool_method, list_keys_method, set_key_method, unset_key_method,
                version_prop,
            },
            manager_3_2::{refresh_state_method},
            manager_3_6::stop_pool_method,
        },
        types,
    },
    engine::{Engine, KeyDescription, Lockable, PoolUuid, StoppedPoolsInfo},
};

mod methods;
mod props;

pub use methods::{create_pool_method, start_pool_method};
pub use props::stopped_pools_prop;

pub struct ManagerR8 {
    connection: Arc<Connection>,
    engine: Arc<dyn Engine>,
    manager: Lockable<Arc<RwLock<Manager>>>,
    counter: Arc<AtomicU64>,
}

impl ManagerR8 {
    pub fn new(
        engine: Arc<dyn Engine>,
        connection: Arc<Connection>,
        manager: Lockable<Arc<RwLock<Manager>>>,
        counter: Arc<AtomicU64>,
    ) -> Self {
        ManagerR8 {
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

#[interface(name = "org.storage.stratis3.Manager.r8")]
impl ManagerR8 {
    #[zbus(property(emits_changed_signal = "const"))]
    #[allow(non_snake_case)]
    #[allow(clippy::unused_self)]
    fn Version(&self) -> &str {
        version_prop()
    }

    #[zbus(property(emits_changed_signal = "true"))]
    #[allow(non_snake_case)]
    async fn StoppedPools(&self) -> types::ManagerR8<StoppedPoolsInfo> {
        stopped_pools_prop(&self.engine).await
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
        devs: Vec<PathBuf>,
        key_desc: Vec<((bool, u32), KeyDescription)>,
        clevis_info: Vec<((bool, u32), &str, &str)>,
        journal_size: (bool, u64),
        tag_spec: (bool, &str),
        allocate_superblock: (bool, bool),
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
            journal_size,
            tag_spec,
            allocate_superblock,
        )
        .await
    }

    #[allow(non_snake_case)]
    async fn DestroyPool(&self, pool: ObjectPath<'_>) -> ((bool, String), u16, String) {
        destroy_pool_method(&self.engine, &self.connection, &self.manager, pool).await
    }

    #[allow(non_snake_case)]
    async fn StartPool(
        &self,
        id: &str,
        id_type: &str,
        unlock_method: (bool, (bool, u32)),
        key_fd: (bool, Fd<'_>),
    ) -> (
        (
            bool,
            (ObjectPath<'_>, Vec<ObjectPath<'_>>, Vec<ObjectPath<'_>>),
        ),
        u16,
        String,
    ) {
        start_pool_method(
            &self.engine,
            &self.connection,
            &self.manager,
            &self.counter,
            id,
            id_type,
            unlock_method,
            key_fd,
        )
        .await
    }

    #[allow(non_snake_case)]
    async fn StopPool(&self, id: &str, id_type: &str) -> ((bool, PoolUuid), u16, String) {
        stop_pool_method(&self.engine, &self.connection, &self.manager, id, id_type).await
    }

    #[allow(non_snake_case)]
    async fn RefreshState(&self) -> (u16, String) {
        refresh_state_method(&self.engine).await
    }
}

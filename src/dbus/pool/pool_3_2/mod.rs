// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc},
};

use tokio::sync::RwLock;
use zbus::{
    fdo::Error,
    interface,
    zvariant::{ObjectPath, OwnedObjectPath},
    Connection,
};

use crate::{
    dbus::{
        manager::Manager,
        pool::{
            pool_3_0::{
                add_cache_devs_method, add_data_devs_method, allocated_prop,
                avail_actions_property, bind_clevis_method, bind_keyring_method,
                clevis_info_property, create_filesystems_method, destroy_filesystems_method,
                encrypted_prop, has_cache_property, init_cache_method, key_description_property,
                name_prop, rebind_clevis_method, rebind_keyring_method, set_name_method, size_prop,
                snapshot_filesystem_method, unbind_clevis_method, unbind_keyring_method, used_prop,
            },
            pool_3_1::{
                enable_overprovisioning_prop, fs_limit_prop, no_alloc_space_prop,
                set_enable_overprovisioning_prop, set_fs_limit_prop,
            },
            shared::{pool_prop, set_pool_prop, try_pool_prop},
        },
    },
    engine::{self, ActionAvailability, Engine, KeyDescription, Lockable, PoolUuid},
    stratis::StratisResult,
};

pub struct PoolR2 {
    connection: Arc<Connection>,
    engine: Arc<dyn Engine>,
    manager: Lockable<Arc<RwLock<Manager>>>,
    counter: Arc<AtomicU64>,
    uuid: PoolUuid,
}

impl PoolR2 {
    fn new(
        engine: Arc<dyn Engine>,
        connection: Arc<Connection>,
        manager: Lockable<Arc<RwLock<Manager>>>,
        counter: Arc<AtomicU64>,
        uuid: PoolUuid,
    ) -> Self {
        PoolR2 {
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

    pub async fn unregister(
        connection: &Arc<Connection>,
        path: ObjectPath<'_>,
    ) -> StratisResult<()> {
        connection.object_server().remove::<PoolR2, _>(path).await?;
        Ok(())
    }
}

#[interface(name = "org.storage.stratis3.pool.r2")]
impl PoolR2 {
    async fn create_filesystems(
        &self,
        specs: Vec<(&str, (bool, &str))>,
    ) -> ((bool, Vec<OwnedObjectPath>), u16, String) {
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

    async fn destroy_filesystems(
        &self,
        filesystems: Vec<ObjectPath<'_>>,
    ) -> ((bool, Vec<String>), u16, String) {
        destroy_filesystems_method(
            &self.engine,
            &self.connection,
            &self.manager,
            self.uuid,
            filesystems,
        )
        .await
    }

    async fn snapshot_filesystem(
        &self,
        origin: ObjectPath<'_>,
        snapshot_name: String,
    ) -> ((bool, OwnedObjectPath), u16, String) {
        snapshot_filesystem_method(
            &self.engine,
            &self.connection,
            &self.manager,
            &self.counter,
            self.uuid,
            origin,
            snapshot_name,
        )
        .await
    }

    async fn add_data_devs(
        &self,
        devices: Vec<PathBuf>,
    ) -> ((bool, Vec<OwnedObjectPath>), u16, String) {
        add_data_devs_method(
            &self.engine,
            &self.connection,
            &self.manager,
            &self.counter,
            self.uuid,
            devices,
        )
        .await
    }

    async fn init_cache(
        &self,
        devices: Vec<PathBuf>,
    ) -> ((bool, Vec<OwnedObjectPath>), u16, String) {
        init_cache_method(
            &self.engine,
            &self.connection,
            &self.manager,
            &self.counter,
            self.uuid,
            devices,
        )
        .await
    }

    async fn add_cache_devs(
        &self,
        devices: Vec<PathBuf>,
    ) -> ((bool, Vec<OwnedObjectPath>), u16, String) {
        add_cache_devs_method(
            &self.engine,
            &self.connection,
            &self.manager,
            &self.counter,
            self.uuid,
            devices,
        )
        .await
    }

    async fn set_name(&self, name: &str) -> ((bool, String), u16, String) {
        set_name_method(
            &self.engine,
            &self.connection,
            &self.manager,
            self.uuid,
            name,
        )
        .await
    }

    async fn bind_clevis(&self, pin: String, json: &str) -> (bool, u16, String) {
        bind_clevis_method(
            &self.engine,
            &self.connection,
            &self.manager,
            self.uuid,
            pin,
            json,
        )
        .await
    }

    async fn bind_keyring(&self, key_desc: KeyDescription) -> (bool, u16, String) {
        bind_keyring_method(
            &self.engine,
            &self.connection,
            &self.manager,
            self.uuid,
            key_desc,
        )
        .await
    }

    async fn rebind_clevis(&self) -> (bool, u16, String) {
        rebind_clevis_method(&self.engine, &self.connection, &self.manager, self.uuid).await
    }

    async fn rebind_keyring(&self, key_desc: KeyDescription) -> (bool, u16, String) {
        rebind_keyring_method(
            &self.engine,
            &self.connection,
            &self.manager,
            self.uuid,
            key_desc,
        )
        .await
    }

    async fn unbind_clevis(&self) -> (bool, u16, String) {
        unbind_clevis_method(&self.engine, &self.connection, &self.manager, self.uuid).await
    }

    async fn unbind_keyring(&self) -> (bool, u16, String) {
        unbind_keyring_method(&self.engine, &self.connection, &self.manager, self.uuid).await
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn uuid(&self) -> PoolUuid {
        self.uuid
    }

    #[zbus(property(emits_changed_signal = "true"))]
    async fn name(&self) -> Result<engine::Name, Error> {
        pool_prop(&self.engine, self.uuid, name_prop).await
    }

    #[zbus(property(emits_changed_signal = "const"))]
    async fn encrypted(&self) -> Result<bool, Error> {
        pool_prop(&self.engine, self.uuid, encrypted_prop).await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    async fn available_actions(&self) -> Result<ActionAvailability, Error> {
        pool_prop(&self.engine, self.uuid, avail_actions_property).await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    async fn key_description(&self) -> Result<(bool, (bool, String)), Error> {
        pool_prop(&self.engine, self.uuid, key_description_property).await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    async fn clevis_info(&self) -> Result<(bool, (bool, (String, String))), Error> {
        pool_prop(&self.engine, self.uuid, clevis_info_property).await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    async fn has_cache(&self) -> Result<bool, Error> {
        pool_prop(&self.engine, self.uuid, has_cache_property).await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    async fn total_physical_size(&self) -> Result<String, Error> {
        pool_prop(&self.engine, self.uuid, size_prop).await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    async fn total_physical_used(&self) -> Result<(bool, String), Error> {
        try_pool_prop(&self.engine, self.uuid, used_prop).await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    async fn allocated_size(&self) -> Result<String, Error> {
        pool_prop(&self.engine, self.uuid, allocated_prop).await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    async fn fs_limit(&self) -> Result<u64, Error> {
        pool_prop(&self.engine, self.uuid, fs_limit_prop).await
    }

    #[zbus(property)]
    async fn set_fs_limit(&self, fs_limit: u64) -> Result<(), zbus::Error> {
        set_pool_prop(&self.engine, self.uuid, set_fs_limit_prop, fs_limit).await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    async fn overprovisioning(&self) -> Result<bool, Error> {
        pool_prop(&self.engine, self.uuid, enable_overprovisioning_prop).await
    }

    #[zbus(property)]
    async fn set_overprovisioning(&self, overprov: bool) -> Result<(), zbus::Error> {
        set_pool_prop(
            &self.engine,
            self.uuid,
            set_enable_overprovisioning_prop,
            overprov,
        )
        .await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    async fn no_alloc_space(&self) -> Result<bool, Error> {
        pool_prop(&self.engine, self.uuid, no_alloc_space_prop).await
    }
}

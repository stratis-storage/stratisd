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
    zvariant::{ObjectPath, OwnedObjectPath, Value},
    Connection,
};

use crate::{
    dbus::{
        manager::Manager,
        pool::{
            pool_3_0::{
                add_cache_devs_method, add_data_devs_method, allocated_prop,
                avail_actions_property, destroy_filesystems_method, encrypted_prop,
                has_cache_property, name_prop, set_name_method, size_prop,
                snapshot_filesystem_method, used_prop,
            },
            pool_3_1::{
                enable_overprovisioning_prop, fs_limit_prop, no_alloc_space_prop,
                set_enable_overprovisioning_prop, set_fs_limit_prop,
            },
            pool_3_3::grow_physical_device_method,
            pool_3_5::init_cache_method,
            pool_3_6::create_filesystems_method,
            pool_3_7::{filesystem_metadata_method, metadata_method},
            shared::{pool_prop, set_pool_prop, try_pool_prop},
        },
        types::FilesystemSpec,
    },
    engine::{self, ActionAvailability, DevUuid, Engine, KeyDescription, Lockable, PoolUuid},
    stratis::StratisResult,
};

mod methods;
mod props;

pub use methods::{
    bind_clevis_method, bind_keyring_method, rebind_clevis_method, rebind_keyring_method,
    unbind_clevis_method, unbind_keyring_method,
};
pub use props::{
    clevis_infos_prop, free_token_slots_prop, key_descs_prop, metadata_version_prop,
    volume_key_loaded_prop,
};

pub struct PoolR8 {
    connection: Arc<Connection>,
    engine: Arc<dyn Engine>,
    manager: Lockable<Arc<RwLock<Manager>>>,
    counter: Arc<AtomicU64>,
    uuid: PoolUuid,
}

impl PoolR8 {
    fn new(
        engine: Arc<dyn Engine>,
        connection: Arc<Connection>,
        manager: Lockable<Arc<RwLock<Manager>>>,
        counter: Arc<AtomicU64>,
        uuid: PoolUuid,
    ) -> Self {
        PoolR8 {
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
        connection.object_server().remove::<PoolR8, _>(path).await?;
        Ok(())
    }
}

#[interface(name = "org.storage.stratis3.pool.r8")]
impl PoolR8 {
    async fn create_filesystems(
        &self,
        specs: FilesystemSpec<'_>,
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

    async fn set_name(&self, name: &str) -> ((bool, PoolUuid), u16, String) {
        set_name_method(
            &self.engine,
            &self.connection,
            &self.manager,
            self.uuid,
            name,
        )
        .await
    }

    async fn bind_clevis(
        &self,
        pin: String,
        json: &str,
        token_slot: (bool, u32),
    ) -> (bool, u16, String) {
        bind_clevis_method(
            &self.engine,
            &self.connection,
            &self.manager,
            self.uuid,
            pin,
            json,
            token_slot,
        )
        .await
    }

    async fn bind_keyring(
        &self,
        key_desc: KeyDescription,
        token_slot: (bool, u32),
    ) -> (bool, u16, String) {
        bind_keyring_method(
            &self.engine,
            &self.connection,
            &self.manager,
            self.uuid,
            key_desc,
            token_slot,
        )
        .await
    }

    async fn rebind_clevis(&self, token_slot: (bool, u32)) -> (bool, u16, String) {
        rebind_clevis_method(
            &self.engine,
            &self.connection,
            &self.manager,
            self.uuid,
            token_slot,
        )
        .await
    }

    async fn rebind_keyring(
        &self,
        key_desc: KeyDescription,
        token_slot: (bool, u32),
    ) -> (bool, u16, String) {
        rebind_keyring_method(
            &self.engine,
            &self.connection,
            &self.manager,
            self.uuid,
            key_desc,
            token_slot,
        )
        .await
    }

    async fn unbind_clevis(&self, token_slot: (bool, u32)) -> (bool, u16, String) {
        unbind_clevis_method(
            &self.engine,
            &self.connection,
            &self.manager,
            self.uuid,
            token_slot,
        )
        .await
    }

    async fn unbind_keyring(&self, token_slot: (bool, u32)) -> (bool, u16, String) {
        unbind_keyring_method(
            &self.engine,
            &self.connection,
            &self.manager,
            self.uuid,
            token_slot,
        )
        .await
    }

    async fn grow_physical_device(&self, dev: DevUuid) -> (bool, u16, String) {
        grow_physical_device_method(
            &self.engine,
            &self.connection,
            &self.manager,
            self.uuid,
            dev,
        )
        .await
    }

    async fn metadata(&self, current: bool) -> (String, u16, String) {
        metadata_method(&self.engine, self.uuid, current).await
    }

    async fn filesystem_metadata(
        &self,
        fs_name: (bool, &str),
        current: bool,
    ) -> (String, u16, String) {
        filesystem_metadata_method(&self.engine, self.uuid, fs_name, current).await
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
    async fn key_descriptions(&self) -> Result<Value<'_>, Error> {
        pool_prop(&self.engine, self.uuid, key_descs_prop).await
    }

    #[zbus(property(emits_changed_signal = "true"))]
    async fn clevis_infos(&self) -> Result<Value<'_>, Error> {
        pool_prop(&self.engine, self.uuid, clevis_infos_prop).await
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

    #[zbus(property(emits_changed_signal = "true"))]
    async fn free_token_slots(&self) -> Result<(bool, u8), Error> {
        pool_prop(&self.engine, self.uuid, free_token_slots_prop).await
    }

    #[zbus(property(emits_changed_signal = "false"))]
    async fn volume_key_loaded(&self) -> Result<Value<'_>, Error> {
        pool_prop(&self.engine, self.uuid, volume_key_loaded_prop).await
    }

    #[zbus(property(emits_changed_signal = "false"))]
    async fn metadata_version(&self) -> Result<u64, Error> {
        pool_prop(&self.engine, self.uuid, metadata_version_prop).await
    }
}

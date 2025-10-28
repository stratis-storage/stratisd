// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc},
};

use zbus::{interface, zvariant::ObjectPath, Connection, Result};

use crate::{
    dbus::{
        consts,
        manager::{manager_3_0::version_prop, manager_3_8::create_pool_method},
    },
    engine::{Engine, KeyDescription},
};

pub struct ManagerR9 {
    connection: Arc<Connection>,
    engine: Arc<dyn Engine>,
    counter: Arc<AtomicU64>,
}

impl ManagerR9 {
    pub fn new(
        connection: Arc<Connection>,
        engine: Arc<dyn Engine>,
        counter: Arc<AtomicU64>,
    ) -> Self {
        ManagerR9 {
            connection,
            engine,
            counter,
        }
    }

    pub async fn register(
        connection: &Arc<Connection>,
        engine: Arc<dyn Engine>,
        counter: Arc<AtomicU64>,
    ) -> Result<()> {
        let manager = Self::new(Arc::clone(connection), Arc::clone(&engine), counter);
        connection
            .object_server()
            .at(consts::STRATIS_BASE_PATH, manager)
            .await?;
        Ok(())
    }
}

#[interface(name = "org.storage.stratis3.Manager.r9")]
impl ManagerR9 {
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
            &self.connection,
            &self.engine,
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

    #[zbus(property(emits_changed_signal = "const"))]
    #[allow(non_snake_case)]
    #[allow(clippy::unused_self)]
    fn Version(&self) -> &str {
        version_prop()
    }
}

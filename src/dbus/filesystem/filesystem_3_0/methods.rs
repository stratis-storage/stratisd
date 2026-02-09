// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use tokio::sync::RwLock;
use zbus::Connection;

use crate::{
    dbus::{consts::OK_STRING, manager::Manager, types::DbusErrorEnum, util::send_fs_name_signal},
    engine::{Engine, FilesystemUuid, Lockable, PoolIdentifier, PoolUuid, RenameAction},
    stratis::StratisError,
};

pub async fn set_name_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    uuid: PoolUuid,
    fs_uuid: FilesystemUuid,
    name: &str,
) -> ((bool, String), u16, String) {
    let default_return = (false, FilesystemUuid::nil().simple().to_string());

    let result = {
        let mut guard = match engine
            .get_mut_pool(PoolIdentifier::Uuid(uuid))
            .await
            .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {uuid}")))
        {
            Err(err) => return (default_return, DbusErrorEnum::ERROR as u16, err.to_string()),
            Ok(guard) => guard,
        };

        let (pool_name, _, pool) = guard.as_mut_tuple();
        handle_action!(pool.rename_filesystem(&pool_name, fs_uuid, name))
    };

    match result {
        Ok(RenameAction::NoSource) => (
            default_return,
            DbusErrorEnum::ERROR as u16,
            format!("pool doesn't know about filesystem {fs_uuid}"),
        ),
        Ok(RenameAction::Renamed(_)) => {
            match manager.read().await.filesystem_get_path(&fs_uuid) {
                Some(p) => {
                    send_fs_name_signal(connection, &p.as_ref()).await;
                }
                None => {
                    warn!("No object path associated with pool UUID {uuid}; failed to send pool name change signals");
                }
            };
            (
                (true, fs_uuid.simple().to_string()),
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            )
        }
        Ok(RenameAction::Identity) => (
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Err(err) => (default_return, DbusErrorEnum::ERROR as u16, err.to_string()),
    }
}

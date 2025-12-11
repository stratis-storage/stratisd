// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use crate::{
    dbus::{consts::OK_STRING, types::DbusErrorEnum},
    engine::{Engine, FilesystemUuid, PoolIdentifier, PoolUuid, RenameAction},
    stratis::StratisError,
};

pub async fn set_name_method(
    engine: &Arc<dyn Engine>,
    uuid: PoolUuid,
    fs_uuid: FilesystemUuid,
    name: &str,
) -> ((bool, FilesystemUuid), u16, String) {
    let default_return = (false, FilesystemUuid::nil());

    match engine
        .get_mut_pool(PoolIdentifier::Uuid(uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {uuid}")))
    {
        Err(err) => (default_return, DbusErrorEnum::ERROR as u16, err.to_string()),
        Ok(mut guard) => {
            let (pool_name, _, pool) = guard.as_mut_tuple();
            match handle_action!(pool.rename_filesystem(&pool_name, fs_uuid, name)) {
                Ok(RenameAction::NoSource) => (
                    default_return,
                    DbusErrorEnum::ERROR as u16,
                    format!("pool doesn't know about filesystem {fs_uuid}"),
                ),
                Ok(RenameAction::Renamed(_)) => (
                    // FIXME: send signal
                    (true, fs_uuid),
                    DbusErrorEnum::OK as u16,
                    OK_STRING.to_string(),
                ),
                Ok(RenameAction::Identity) => (
                    default_return,
                    DbusErrorEnum::OK as u16,
                    OK_STRING.to_string(),
                ),
                Err(err) => (default_return, DbusErrorEnum::ERROR as u16, err.to_string()),
            }
        }
    }
}

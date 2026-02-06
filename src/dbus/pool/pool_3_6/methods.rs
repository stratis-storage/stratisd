// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{atomic::AtomicU64, Arc};

use tokio::sync::RwLock;
use zbus::{zvariant::OwnedObjectPath, Connection};

use devicemapper::Bytes;

use crate::{
    dbus::{
        consts::OK_STRING,
        filesystem::register_filesystem,
        manager::Manager,
        types::{DbusErrorEnum, FilesystemSpec},
        util::{engine_to_dbus_err_tuple, tuple_to_option},
    },
    engine::{Engine, EngineAction, Lockable, PoolIdentifier, PoolUuid},
    stratis::StratisError,
};

#[allow(clippy::too_many_arguments)]
pub async fn create_filesystems_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    pool_uuid: PoolUuid,
    filesystems: FilesystemSpec<'_>,
) -> ((bool, Vec<(OwnedObjectPath, String)>), u16, String) {
    let default_return = (false, (Vec::new()));

    if filesystems.len() > 1 {
        return (
            default_return,
            DbusErrorEnum::ERROR as u16,
            "Currently filesystem creation is limited to one filesystem at a time".to_string(),
        );
    }

    let filesystem_specs = match filesystems
        .into_iter()
        .map(|(name, size_opt, size_limit_opt)| {
            let size = tuple_to_option(size_opt)
                .map(|val| {
                    val.parse::<u128>().map_err(|_| {
                        format!("Could not parse filesystem size string {val} to integer value")
                    })
                })
                .transpose()?;
            let size_limit = tuple_to_option(size_limit_opt)
                .map(|val| {
                    val.parse::<u128>().map_err(|_| {
                        format!(
                            "Could not parse filesystem size limit string {val} to integer value"
                        )
                    })
                })
                .transpose()?;
            Ok((name.to_string(), size.map(Bytes), size_limit.map(Bytes)))
        })
        .collect::<Result<Vec<(String, Option<Bytes>, Option<Bytes>)>, String>>()
    {
        Ok(fs_specs) => fs_specs,
        Err(e) => {
            let (rc, rs) = (DbusErrorEnum::ERROR as u16, e);
            return (default_return, rc, rs);
        }
    };

    let guard_res = engine
        .get_mut_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;
        let (name, _, pool) = guard.as_mut_tuple();
        handle_action!(pool.create_filesystems(
            name.to_string().as_str(),
            pool_uuid,
            filesystem_specs
                .iter()
                .map(|(s, b1, b2)| (s.as_str(), *b1, *b2))
                .collect::<Vec<_>>()
                .as_slice(),
        ))
    })
    .await
    {
        Ok(Ok(changed)) => {
            let mut object_paths = Vec::new();
            match changed.changed() {
                Some(v) => {
                    for (name, uuid, _) in v {
                        match register_filesystem(
                            engine, connection, manager, counter, pool_uuid, uuid,
                        )
                        .await
                        {
                            Ok(path) => {
                                object_paths.push((OwnedObjectPath::from(path), name.to_string()));
                            }
                            Err(e) => {
                                warn!("Failed to register the filesystem with the D-Bus: {e}; object may not be visible to clients");
                            }
                        }
                    }
                    (
                        (true, object_paths),
                        DbusErrorEnum::OK as u16,
                        OK_STRING.to_string(),
                    )
                }
                None => (
                    default_return,
                    DbusErrorEnum::OK as u16,
                    OK_STRING.to_string(),
                ),
            }
        }
        Ok(Err(e)) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            (default_return, rc, rs)
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::from(e));
            (default_return, rc, rs)
        }
    }
}

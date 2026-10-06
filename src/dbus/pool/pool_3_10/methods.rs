// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc},
};

use serde_json::{from_str, Value};
use tokio::sync::RwLock;
use zbus::{zvariant::OwnedObjectPath, Connection};

use devicemapper::Bytes;

use crate::{
    dbus::{
        blockdev::{register_blockdev, unregister_blockdev},
        consts::OK_STRING,
        manager::Manager,
        types::DbusErrorEnum,
        util::{
            engine_to_dbus_err_tuple, send_action_availability_signal, send_clevis_info_signal,
            send_encrypted_signal, send_free_token_slots_signal, send_has_cache_signal,
            send_keyring_signal, send_last_reencrypted_signal, send_pool_foreground_signals,
            tuple_to_option,
        },
    },
    engine::{
        Engine, EngineAction, InputEncryptionInfo, IntegritySpec, IntegrityTagSpec, KeyDescription,
        Lockable, PoolIdentifier, PoolUuid,
    },
    stratis::{StratisError, StratisResult},
};

pub async fn remove_cache_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    pool_uuid: PoolUuid,
) -> ((bool, Vec<String>), u16, String) {
    let default_return = (false, Vec::default());

    let guard_res = engine
        .get_mut_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    let conn_clone = Arc::clone(connection);
    let man_clone = manager.clone();
    match tokio::task::spawn_blocking(move || {
        let mut guard = guard_res?;
        let (name, _, pool) = guard.as_mut_tuple();
        handle_action!(
            pool.remove_cache(pool_uuid, name.to_string().as_str()),
            conn_clone,
            man_clone,
            pool_uuid
        )
    })
    .await
    {
        Ok(Ok(action)) => match action.changed() {
            Some((dev_uuids, _)) => {
                match manager.read().await.pool_get_path(&pool_uuid) {
                    Some(p) => {
                        send_has_cache_signal(connection, p).await;
                    }
                    None => {
                        warn!("No object path associated with pool UUID {pool_uuid}; failed to send pool has cache change signals");
                    }
                };

                let mut removed_uuids = Vec::new();
                let mut failed_unregisters = Vec::new();
                for dev_uuid in dev_uuids {
                    let opt = manager.read().await.blockdev_get_path(&dev_uuid).cloned();
                    match opt {
                        Some(p) => {
                            if let Err(e) =
                                unregister_blockdev(connection, manager, &p.as_ref()).await
                            {
                                failed_unregisters.push(e);
                            } else {
                                removed_uuids.push(dev_uuid.simple().to_string());
                            }
                        }
                        None => {
                            warn!("No path found to unregister for removed cache blockdev with UUID {dev_uuid}");
                        }
                    }
                }
                if failed_unregisters.is_empty() {
                    (
                        (true, removed_uuids),
                        DbusErrorEnum::OK as u16,
                        OK_STRING.to_string(),
                    )
                } else {
                    let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::BestEffortError(
                        "Failed to unregister all blockdevs from the D-Bus".to_string(),
                        failed_unregisters,
                    ));
                    (default_return, rc, rs)
                }
            }
            None => (
                default_return,
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            ),
        },
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

#[allow(clippy::too_many_arguments)]
pub async fn migrate_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    pool_uuid: PoolUuid,
    devices: Vec<PathBuf>,
    key_descs: Vec<((bool, u32), KeyDescription)>,
    clevis_infos: Vec<((bool, u32), &str, &str)>,
    journal_size: (bool, u64),
    tag_spec: (bool, &str),
    allocate_superblock: (bool, bool),
) -> ((bool, Vec<OwnedObjectPath>), u16, String) {
    let default_return = (false, Vec::new());

    let key_descs = key_descs
        .into_iter()
        .map(|(tup, kd)| (tuple_to_option(tup), kd))
        .collect::<Vec<_>>();
    let clevis_infos = match clevis_infos.into_iter().try_fold::<_, _, StratisResult<_>>(
        Vec::new(),
        |mut vec, (tup, pin, json)| {
            vec.push((
                tuple_to_option(tup),
                (
                    pin.to_string(),
                    from_str::<Value>(json).map_err(StratisError::from)?,
                ),
            ));
            Ok(vec)
        },
    ) {
        Ok(ci) => ci,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return (default_return, rc, rs);
        }
    };
    let iei = match InputEncryptionInfo::new(key_descs, clevis_infos) {
        Ok(info) => info,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return (default_return, rc, rs);
        }
    };
    let journal_size = tuple_to_option(journal_size).map(Bytes::from);
    let tag_spec = match tuple_to_option(tag_spec)
        .map(IntegrityTagSpec::try_from)
        .transpose()
    {
        Ok(s) => s,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Msg(format!(
                "Failed to parse integrity tag specification: {e}"
            )));
            return (default_return, rc, rs);
        }
    };
    let allocate_superblock = tuple_to_option(allocate_superblock);

    // The pool keeps its name and UUID across a migration, but all of its
    // block devices are replaced, so record the devices that will have to be
    // removed from the D-Bus tree once the migration has succeeded.
    let (pool_name, old_dev_uuids) = match engine.get_pool(PoolIdentifier::Uuid(pool_uuid)).await {
        Some(guard) => {
            let (name, _, pool) = guard.as_tuple();
            (
                name.to_string(),
                pool.blockdevs()
                    .into_iter()
                    .map(|(u, _, _)| u)
                    .collect::<Vec<_>>(),
            )
        }
        None => {
            let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Msg(format!(
                "No pool associated with uuid {pool_uuid}"
            )));
            return (default_return, rc, rs);
        }
    };

    let devs = devices
        .iter()
        .map(|path| path.as_path())
        .collect::<Vec<_>>();
    match engine
        .migrate(
            pool_uuid,
            pool_name.as_str(),
            devs.as_slice(),
            iei.as_ref(),
            IntegritySpec {
                journal_size,
                tag_spec,
                allocate_superblock,
            },
        )
        .await
    {
        Ok(()) => {
            info!("Pool with UUID {pool_uuid} was successfully migrated");

            for dev_uuid in old_dev_uuids {
                let opt = manager.read().await.blockdev_get_path(&dev_uuid).cloned();
                match opt {
                    Some(p) => {
                        if let Err(e) = unregister_blockdev(connection, manager, &p.as_ref()).await
                        {
                            warn!("Failed to unregister {p} representing blockdev {dev_uuid} that is no longer part of pool {pool_uuid}: {e}");
                        }
                    }
                    None => {
                        warn!("No path found to unregister for migrated blockdev with UUID {dev_uuid}");
                    }
                }
            }

            let new_dev_uuids = match engine.get_pool(PoolIdentifier::Uuid(pool_uuid)).await {
                Some(guard) => {
                    let (_, _, pool) = guard.as_tuple();
                    pool.blockdevs()
                        .into_iter()
                        .map(|(u, _, _)| u)
                        .collect::<Vec<_>>()
                }
                None => {
                    let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Msg(format!(
                        "Pool with UUID {pool_uuid} was successfully migrated but appears to have been removed before its new block devices could be exposed on the D-Bus"
                    )));
                    return (default_return, rc, rs);
                }
            };

            let mut bd_paths = Vec::new();
            for dev_uuid in new_dev_uuids {
                match register_blockdev(engine, connection, manager, counter, pool_uuid, dev_uuid)
                    .await
                {
                    Ok(op) => bd_paths.push(OwnedObjectPath::from(op)),
                    Err(_) => {
                        warn!("Unable to register object path for blockdev with UUID {dev_uuid} belonging to pool {pool_uuid} on the D-Bus");
                    }
                }
            }

            // A migration builds an entirely new backstore for the pool, so
            // pick up the resulting size changes and signal them.
            let mut diffs = engine.pool_evented(Some(&HashSet::from([pool_uuid]))).await;
            if let Some(diff) = diffs.remove(&pool_uuid) {
                send_pool_foreground_signals(connection, manager, pool_uuid, diff).await;
            }

            // The remaining backstore properties are not covered by the pool
            // diff and have to be signalled individually. Properties that
            // belong to the pool rather than the backstore, like the
            // filesystem limit, are unaffected by a migration.
            match manager.read().await.pool_get_path(&pool_uuid) {
                Some(p) => {
                    send_has_cache_signal(connection, p).await;
                    send_encrypted_signal(connection, p).await;
                    send_keyring_signal(connection, p, true).await;
                    send_clevis_info_signal(connection, p, true).await;
                    send_free_token_slots_signal(connection, p).await;
                    send_last_reencrypted_signal(connection, p).await;
                }
                None => {
                    warn!("No object path associated with pool UUID {pool_uuid}; failed to send pool property change signals");
                }
            };

            (
                (true, bd_paths),
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            )
        }
        Err(e) => {
            warn!("Migration of pool with UUID {pool_uuid} failed: {e}");
            if e.error_to_available_actions().is_some() {
                match manager.read().await.pool_get_path(&pool_uuid) {
                    Some(p) => {
                        send_action_availability_signal(connection, p).await;
                    }
                    None => {
                        warn!("Could not find path associated with pool with UUID {pool_uuid}; could not send action availability change signal");
                    }
                }
            }
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            (default_return, rc, rs)
        }
    }
}

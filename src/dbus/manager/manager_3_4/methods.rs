// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{atomic::AtomicU64, Arc};

use tokio::sync::RwLock;
use zbus::{zvariant::OwnedObjectPath, Connection};

use crate::{
    dbus::{
        consts::OK_STRING,
        manager::Manager,
        pool::register_pool,
        types::DbusErrorEnum,
        util::{
            engine_to_dbus_err_tuple, send_locked_pools_signals, send_stopped_pools_signals,
            tuple_to_option,
        },
    },
    engine::{
        Engine, Lockable, Name, PoolIdentifier, PoolUuid, StartAction, TokenUnlockMethod,
        UnlockMethod,
    },
};

pub async fn start_pool_method(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    counter: &Arc<AtomicU64>,
    id: &str,
    id_type: &str,
    unlock_method_tuple: (bool, UnlockMethod),
) -> (
    (
        bool,
        (OwnedObjectPath, Vec<OwnedObjectPath>, Vec<OwnedObjectPath>),
    ),
    u16,
    String,
) {
    let default_return = (
        false,
        (OwnedObjectPath::default(), Vec::default(), Vec::default()),
    );

    let id = match id_type {
        "uuid" => match PoolUuid::parse_str(id) {
            Ok(u) => PoolIdentifier::Uuid(u),
            Err(e) => {
                let (rc, rs) = engine_to_dbus_err_tuple(&e);
                return (default_return, rc, rs);
            }
        },
        "name" => PoolIdentifier::Name(Name::new(id.to_string())),
        _ => {
            return (
                default_return,
                DbusErrorEnum::ERROR as u16,
                format!("ID type {id_type} not recognized"),
            );
        }
    };
    let unlock_method = tuple_to_option(unlock_method_tuple);

    match handle_action!(
        engine
            .start_pool(id, TokenUnlockMethod::from(unlock_method), None, false)
            .await
    ) {
        Ok(StartAction::Started(pool_uuid)) => {
            let guard = match engine.get_pool(PoolIdentifier::Uuid(pool_uuid)).await {
                Some(g) => g,
                None => {
                    return (
                        default_return,
                        DbusErrorEnum::ERROR as u16,
                        format!("No pool found for newly started pool with UUID {pool_uuid}"),
                    );
                }
            };
            let (pool_path, fs_paths, dev_paths) =
                match register_pool(engine, connection, manager, counter, pool_uuid).await {
                    Ok((pp, fp, dp)) => (
                        OwnedObjectPath::from(pp),
                        fp.into_iter().map(OwnedObjectPath::from).collect(),
                        dp.into_iter().map(OwnedObjectPath::from).collect(),
                    ),
                    Err(e) => {
                        let (rc, rs) = engine_to_dbus_err_tuple(&e);
                        return (default_return, rc, rs);
                    }
                };

            if guard.is_encrypted() {
                send_locked_pools_signals(connection).await;
            }
            send_stopped_pools_signals(connection).await;

            (
                (true, (pool_path, dev_paths, fs_paths)),
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            )
        }
        Ok(StartAction::Identity) => (
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            (default_return, rc, rs)
        }
    }
}

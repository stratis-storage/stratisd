// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use chrono::SecondsFormat;
use devicemapper::Bytes;
use either::Either;
use tokio::sync::RwLock;
use zbus::{fdo::Error, zvariant::Value, Connection};

use crate::{
    dbus::{
        manager::Manager,
        util::{option_to_tuple, send_fs_limit_signal, send_overprovisioning_signal},
    },
    engine::{
        ActionAvailability, Lockable, Name, Pool, PoolUuid, SomeLockReadGuard, SomeLockWriteGuard,
    },
};

pub fn name_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> Name {
    let (name, _, _) = guard.as_tuple();
    name
}

pub fn size_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> String {
    (*guard.total_physical_size().bytes()).to_string()
}

pub fn used_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> (bool, String) {
    let (b, used) = option_to_tuple(
        guard.total_physical_used().map(|s| s.bytes()),
        Bytes::from(0u64),
    );
    (b, (*used).to_string())
}

pub fn allocated_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> String {
    (*guard.total_allocated_size().bytes()).to_string()
}

pub fn encrypted_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> bool {
    guard.is_encrypted()
}

pub fn avail_actions_property(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> ActionAvailability {
    guard.avail_actions()
}

pub fn has_cache_property(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> bool {
    guard.has_cache()
}

pub fn fs_limit_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> u64 {
    guard.fs_limit()
}

pub async fn set_fs_limit_prop(
    mut guard: SomeLockWriteGuard<PoolUuid, dyn Pool>,
    uuid: PoolUuid,
    fs_limit: u64,
) -> Result<bool, Error> {
    let (name, _, p) = guard.as_mut_tuple();
    p.set_fs_limit(&name, uuid, fs_limit)
        .map(|_| true)
        .map_err(|e| Error::Failed(e.to_string()))
}

pub async fn send_fs_limit_signal_on_change(
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    uuid: PoolUuid,
) {
    match manager.read().await.pool_get_path(&uuid) {
        Some(p) => send_fs_limit_signal(connection, p).await,
        None => {
            warn!("No path associated with UUID {uuid}; cannot send property changed signal");
        }
    }
}

pub fn enable_overprovisioning_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> bool {
    guard.overprov_enabled()
}

pub async fn set_enable_overprovisioning_prop(
    mut guard: SomeLockWriteGuard<PoolUuid, dyn Pool>,
    _uuid: PoolUuid,
    enable_overprov: bool,
) -> Result<bool, Error> {
    let (name, _, p) = guard.as_mut_tuple();
    p.set_overprov_mode(&name, enable_overprov)
        .map(|_| true)
        .map_err(|e| Error::Failed(e.to_string()))
}

pub async fn send_enable_overprovisioning_signal_on_change(
    connection: &Arc<Connection>,
    manager: &Lockable<Arc<RwLock<Manager>>>,
    uuid: PoolUuid,
) {
    match manager.read().await.pool_get_path(&uuid) {
        Some(p) => send_overprovisioning_signal(connection, p).await,
        None => {
            warn!("No path associated with UUID {uuid}; cannot send property changed signal");
        }
    }
}

pub fn no_alloc_space_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> bool {
    guard.out_of_alloc_space()
}

pub fn free_token_slots_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> (bool, u8) {
    option_to_tuple(guard.free_token_slots(), 0)
}

pub fn metadata_version_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> u64 {
    guard.metadata_version() as u64
}

pub fn volume_key_loaded_prop<'a>(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> Value<'a> {
    let (_, pool_uuid, pool) = guard.as_tuple();
    match pool.volume_key_is_loaded(pool_uuid) {
        Ok(b) => Value::from(b),
        Err(e) => Value::from(e.to_string()),
    }
}

pub fn key_descs_prop<'a>(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> Value<'a> {
    match guard.encryption_info() {
        Some(Either::Left(ei)) => Value::from(
            ei.all_key_descriptions()
                .map(|(i, kd)| (*i, kd.clone()))
                .collect::<Vec<_>>(),
        ),
        Some(Either::Right(pei)) => Value::from(option_to_tuple(
            pei.key_description()
                .map(|opt| {
                    option_to_tuple(
                        opt.map(|kd| kd.as_application_str().to_string()),
                        String::new(),
                    )
                })
                .ok(),
            (false, String::new()),
        )),
        None => Value::from("Unencrypted".to_string()),
    }
}

pub fn clevis_infos_prop<'a>(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> Value<'a> {
    match guard.encryption_info() {
        Some(Either::Left(ei)) => Value::from(
            ei.all_clevis_infos()
                .map(|(i, (pin, json))| (*i, (pin.to_owned(), json.to_string())))
                .collect::<Vec<_>>(),
        ),
        Some(Either::Right(pei)) => Value::from(option_to_tuple(
            pei.clevis_info()
                .map(|opt| {
                    option_to_tuple(
                        opt.map(|(pin, json)| (pin.to_owned(), json.to_string())),
                        (String::new(), String::new()),
                    )
                })
                .ok(),
            (false, (String::new(), String::new())),
        )),
        None => Value::from("Unencrypted".to_string()),
    }
}

pub fn last_reencrypted_timestamp_prop(
    guard: SomeLockReadGuard<PoolUuid, dyn Pool>,
) -> (bool, String) {
    option_to_tuple(
        guard
            .last_reencrypt()
            .map(|t| t.to_rfc3339_opts(SecondsFormat::Secs, true)),
        String::new(),
    )
}

pub fn metadata_used_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> String {
    (*guard.metadata_used().bytes()).to_string()
}

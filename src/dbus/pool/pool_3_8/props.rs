// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use either::Either;
use zbus::Variant;

use crate::{
    dbus::util::option_to_tuple,
    engine::{Pool, PoolUuid, SomeLockReadGuard},
};

pub fn free_token_slots_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> (bool, u8) {
    option_to_tuple(guard.free_token_slots(), 0)
}

pub fn metadata_version_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> u64 {
    guard.metadata_version() as u64
}

pub fn volume_key_loaded_prop<'a>(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> Variant<'a> {
    let (_, pool_uuid, pool) = guard.as_tuple();
    match pool.volume_key_is_loaded(pool_uuid) {
        Ok(b) => Variant::from(b),
        Err(e) => Variant::from(e.to_string()),
    }
}

pub fn key_descs_prop<'a>(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> Variant<'a> {
    match guard.encryption_info() {
        Some(Either::Left(ei)) => Variant::from(
            ei.all_key_descriptions()
                .map(|(i, kd)| (*i, kd.clone()))
                .collect::<Vec<_>>(),
        ),
        Some(Either::Right(pei)) => Variant::from(option_to_tuple(
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
        None => Variant::from("Unencrypted".to_string()),
    }
}

pub fn clevis_infos_prop<'a>(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> Variant<'a> {
    match guard.encryption_info() {
        Some(Either::Left(ei)) => Variant::from(
            ei.all_clevis_infos()
                .map(|(i, (pin, json))| (*i, (pin.to_owned(), json.to_string())))
                .collect::<Vec<_>>(),
        ),
        Some(Either::Right(pei)) => Variant::from(option_to_tuple(
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
        None => Variant::from("Unencrypted".to_string()),
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use devicemapper::Bytes;
use either::Either;

use crate::{
    dbus::util::option_to_tuple,
    engine::{ActionAvailability, Name, Pool, PoolUuid, SomeLockReadGuard},
};

pub fn name_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> Name {
    let (name, _, _) = guard.as_tuple();
    name
}

pub fn size_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> String {
    (*guard.total_physical_size()).to_string()
}

pub fn used_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> (bool, String) {
    let (b, used) = option_to_tuple(
        guard.total_physical_used().map(|s| s.bytes()),
        Bytes::from(0u64),
    );
    (b, (*used).to_string())
}

pub fn allocated_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> String {
    (*guard.total_allocated_size()).to_string()
}

pub fn encrypted_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> bool {
    guard.is_encrypted()
}

pub fn avail_actions_property(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> ActionAvailability {
    guard.avail_actions()
}

pub fn key_description_property(
    guard: SomeLockReadGuard<PoolUuid, dyn Pool>,
) -> (bool, (bool, String)) {
    match guard.encryption_info() {
        Some(Either::Left(ei)) => (
            true,
            option_to_tuple(
                ei.single_key_description()
                    .map(|(_, kd)| kd.as_application_str().to_string()),
                String::new(),
            ),
        ),
        Some(Either::Right(pei)) => option_to_tuple(
            pei.key_description().ok().map(|opt| {
                option_to_tuple(
                    opt.map(|kd| kd.as_application_str().to_string()),
                    String::new(),
                )
            }),
            (false, String::new()),
        ),
        None => (true, (false, String::new())),
    }
}

pub fn clevis_info_property(
    guard: SomeLockReadGuard<PoolUuid, dyn Pool>,
) -> (bool, (bool, (String, String))) {
    match guard.encryption_info() {
        Some(Either::Left(ei)) => (
            true,
            option_to_tuple(
                ei.single_clevis_info()
                    .map(|(_, (pin, json))| (pin.to_owned(), json.to_string())),
                (String::new(), String::new()),
            ),
        ),
        Some(Either::Right(pei)) => option_to_tuple(
            pei.clevis_info().ok().map(|opt| {
                option_to_tuple(
                    opt.map(|(pin, json)| (pin.to_owned(), json.to_string())),
                    (String::new(), String::new()),
                )
            }),
            (false, (String::new(), String::new())),
        ),
        None => (true, (false, (String::new(), String::new()))),
    }
}

pub fn has_cache_property(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> bool {
    guard.has_cache()
}

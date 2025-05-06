// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use chrono::{DateTime, SecondsFormat, Utc};
use dbus::arg::{RefArg, Variant};
use either::Either;

use devicemapper::Bytes;

use crate::{
    dbus_api::util::option_to_tuple,
    engine::{ActionAvailability, EncryptionInfo, PoolEncryptionInfo},
    stratis::StratisResult,
};

/// Convert an encryption information data structure to a
/// D-Bus type.
fn enc_to_prop<F, T>(
    ei: Option<Either<EncryptionInfo, PoolEncryptionInfo>>,
    f: F,
    default: T,
) -> (bool, (bool, T))
where
    F: Fn(Either<EncryptionInfo, PoolEncryptionInfo>) -> StratisResult<Option<T>>,
    T: Clone,
{
    option_to_tuple(
        ei.map(f)
            .transpose()
            .map(|opt| option_to_tuple(opt.and_then(|subopt| subopt), default.clone()))
            .ok(),
        (false, default),
    )
}

/// Fetch the key description and handle converting it into a
/// D-Bus type.
pub fn key_desc_to_prop(
    ei: Option<Either<EncryptionInfo, PoolEncryptionInfo>>,
) -> (bool, (bool, String)) {
    enc_to_prop(
        ei,
        |ei| {
            ei.either(
                |ei| {
                    Ok(ei
                        .single_key_description()
                        .map(|(_, kd)| kd.as_application_str().to_owned()))
                },
                |pei| {
                    pei.key_description()
                        .map(|opt| opt.map(|k| k.as_application_str().to_owned()))
                },
            )
        },
        String::new(),
    )
}

/// Fetch the key descriptions and handle converting it into a
/// D-Bus type.
pub fn key_descs_to_prop(
    eei: Option<Either<EncryptionInfo, PoolEncryptionInfo>>,
) -> Variant<Box<dyn RefArg>> {
    match eei {
        Some(Either::Left(ei)) => Variant(Box::new(
            ei.all_key_descriptions()
                .map(|(i, kd)| (*i, kd.as_application_str().to_string()))
                .collect::<Vec<_>>(),
        )),
        Some(Either::Right(pei)) => Variant(Box::new(option_to_tuple(
            pei.key_description()
                .map(|kd_opt| {
                    option_to_tuple(
                        kd_opt.map(|kd| kd.as_application_str().to_string()),
                        String::new(),
                    )
                })
                .ok(),
            (false, String::new()),
        ))),
        None => Variant(Box::new("Unencrypted".to_string())),
    }
}

/// Fetch the Clevis information and handle converting it into a
/// D-Bus type.
pub fn clevis_info_to_prop(
    ei: Option<Either<EncryptionInfo, PoolEncryptionInfo>>,
) -> (bool, (bool, (String, String))) {
    enc_to_prop(
        ei,
        |ei| {
            ei.either(
                |ei| {
                    Ok(ei
                        .single_clevis_info()
                        .map(|(_, (pin, ci))| (pin.to_owned(), ci.to_string())))
                },
                |pei| {
                    pei.clevis_info()
                        .map(|opt| opt.map(|(pin, ci)| (pin.to_owned(), ci.to_string())))
                },
            )
        },
        (String::new(), String::new()),
    )
}
/// Fetch the Clevis infos and handle converting it into a
/// D-Bus type.
pub fn clevis_infos_to_prop(
    eei: Option<Either<EncryptionInfo, PoolEncryptionInfo>>,
) -> Variant<Box<dyn RefArg>> {
    match eei {
        Some(Either::Left(ei)) => Variant(Box::new(
            ei.all_clevis_infos()
                .map(|(i, (pin, config))| (*i, (pin.to_owned(), config.to_string())))
                .collect::<Vec<_>>(),
        )),
        Some(Either::Right(pei)) => Variant(Box::new(option_to_tuple(
            pei.clevis_info()
                .map(|ci_opt| {
                    option_to_tuple(
                        ci_opt.map(|(pin, config)| (pin.to_owned(), config.to_string())),
                        (String::new(), String::new()),
                    )
                })
                .ok(),
            (false, (String::new(), String::new())),
        ))),
        None => Variant(Box::new("Unencrypted".to_string())),
    }
}

/// Generate D-Bus representation of pool state property.
#[inline]
pub fn avail_actions_to_prop(avail_actions: ActionAvailability) -> String {
    avail_actions.to_string()
}

/// Generate a D-Bus representation of the total size of the pool in bytes.
#[inline]
pub fn pool_size_to_prop(size: Bytes) -> String {
    (*size).to_string()
}

/// Generate a D-Bus representation of the total allocated size of the pool in bytes.
#[inline]
pub fn pool_alloc_to_prop(alloc: Bytes) -> String {
    (*alloc).to_string()
}

/// Generate a D-Bus representation of the total used size of the pool in bytes.
pub fn pool_used_to_prop(used: Option<Bytes>) -> (bool, String) {
    option_to_tuple(used.map(|u| (*u).to_string()), String::new())
}

/// Generate a D-Bus representation of the free token slots in the pool.
pub fn pool_free_token_slots_to_prop(ts: Option<u8>) -> (bool, u8) {
    option_to_tuple(ts, 0)
}

/// Generate a D-Bus representation of the timestamp of the last time when the pool was
/// reencrypted.
pub fn pool_last_reencrypted_timestamp_to_prop(timestamp: Option<DateTime<Utc>>) -> (bool, String) {
    option_to_tuple(
        timestamp.map(|t| t.to_rfc3339_opts(SecondsFormat::Secs, true)),
        String::new(),
    )
}

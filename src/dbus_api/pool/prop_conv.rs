// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use devicemapper::Bytes;

use crate::{
    dbus_api::util::option_to_tuple,
    engine::{ActionAvailability, PoolEncryptionInfo},
    stratis::StratisResult,
};

/// Convert an encryption information data structure to a
/// D-Bus type.
fn enc_to_prop<F, T>(ei: Option<PoolEncryptionInfo>, f: F, default: T) -> (bool, (bool, T))
where
    F: Fn(PoolEncryptionInfo) -> StratisResult<Option<T>>,
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
pub fn key_desc_to_prop(ei: Option<PoolEncryptionInfo>) -> (bool, (bool, String)) {
    enc_to_prop(
        ei,
        |ei| {
            ei.key_description()
                .map(|kd_opt| kd_opt.map(|kd| kd.as_application_str().to_string()))
        },
        String::new(),
    )
}

/// Fetch the Clevis information and handle converting it into a
/// D-Bus type.
pub fn clevis_info_to_prop(ei: Option<PoolEncryptionInfo>) -> (bool, (bool, (String, String))) {
    enc_to_prop(
        ei,
        |ei| {
            ei.clevis_info()
                .map(|ci_opt| ci_opt.map(|(pin, cfg)| (pin.to_owned(), cfg.to_string())))
        },
        (String::new(), String::new()),
    )
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

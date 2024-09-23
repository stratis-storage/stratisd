// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use devicemapper::{Bytes, Sectors};

use crate::{dbus_api::util::option_to_tuple, engine::FilesystemUuid};

/// Generate D-Bus representation of filesystem size property.
#[inline]
pub fn fs_size_to_prop(size: Bytes) -> String {
    (*size).to_string()
}

/// Generate D-Bus representation of used property.
#[inline]
pub fn fs_used_to_prop(used: Option<Bytes>) -> (bool, String) {
    option_to_tuple(used.map(|u| (*u).to_string()), String::new())
}

/// Generate D-Bus representation of filesystem size limit property.
#[inline]
pub fn fs_size_limit_to_prop(limit: Option<Sectors>) -> (bool, String) {
    option_to_tuple(limit.map(|u| (*u.bytes()).to_string()), String::new())
}

/// Generate D-Bus representation of filesystem origin property.
#[inline]
pub fn fs_origin_to_prop(origin: Option<FilesystemUuid>) -> (bool, String) {
    option_to_tuple(origin.map(|u| (uuid_to_string!(u))), String::new())
}

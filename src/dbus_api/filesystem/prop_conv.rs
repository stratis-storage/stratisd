// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use devicemapper::Bytes;

use crate::dbus_api::util::option_to_tuple;

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

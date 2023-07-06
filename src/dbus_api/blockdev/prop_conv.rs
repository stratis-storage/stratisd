// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use devicemapper::Sectors;

use crate::dbus_api::util::option_to_tuple;

/// Generate D-Bus representation of block device new size property.
#[inline]
pub fn blockdev_new_size_to_prop(new_size: Option<Sectors>) -> (bool, String) {
    option_to_tuple(new_size.map(|s| (*s.bytes()).to_string()), String::new())
}

/// Generate D-Bus representation of block device user info property.
#[inline]
pub fn blockdev_user_info_to_prop(user_info: Option<String>) -> (bool, String) {
    option_to_tuple(user_info, String::new())
}

/// Generate D-Bus representation of block device total physical size property.
#[inline]
pub fn blockdev_total_physical_size_to_prop(total_physical_size: Sectors) -> String {
    (*total_physical_size.bytes()).to_string()
}

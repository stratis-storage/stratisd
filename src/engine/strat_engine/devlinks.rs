// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{path::PathBuf, str};

use crate::engine::engine::DEV_PATH;

pub const UEVENT_CHANGE_EVENT: &str = "change";

/// Given a pool name and a filesystem name, return the path it should be
/// available as a device for mounting.
pub fn filesystem_mount_path<T: AsRef<str>>(pool_name: T, fs_name: T) -> PathBuf {
    vec![DEV_PATH, pool_name.as_ref(), fs_name.as_ref()]
        .iter()
        .collect()
}

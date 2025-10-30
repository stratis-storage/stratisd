// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use zbus::zvariant::Str;

use crate::engine::{Filesystem, FilesystemUuid, Name};

pub fn uuid_prop(uuid: FilesystemUuid) -> String {
    uuid.to_string()
}

pub fn name_prop<'a>(name: Name, _: FilesystemUuid, _: &dyn Filesystem) -> Str<'a> {
    Str::from(name.to_string())
}

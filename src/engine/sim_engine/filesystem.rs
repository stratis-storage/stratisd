// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[cfg(feature = "dbus_enabled")]
use dbus;

use rand;

use std::path::PathBuf;

use super::super::engine::Filesystem;

#[derive(Debug)]
pub struct SimFilesystem {
    rand: u32,
    #[cfg(feature = "dbus_enabled")]
    dbus_path: Option<dbus::Path<'static>>,
}

impl SimFilesystem {
    pub fn new() -> SimFilesystem {
        SimFilesystem {
            rand: rand::random::<u32>(),
            #[cfg(feature = "dbus_enabled")]
            dbus_path: None,
        }
    }
}

impl Filesystem for SimFilesystem {
    fn devnode(&self) -> PathBuf {
        ["/dev/stratis", &format!("random-{}", self.rand)]
            .into_iter()
            .collect()
    }

    #[cfg(feature = "dbus_enabled")]
    fn set_dbus_path(&mut self, path: dbus::Path<'static>) -> () {
        self.dbus_path = Some(path)
    }

    #[cfg(feature = "dbus_enabled")]
    fn get_dbus_path(&self) -> &Option<dbus::Path<'static>> {
        &self.dbus_path
    }
}

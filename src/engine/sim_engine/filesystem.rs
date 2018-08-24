// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use chrono::{DateTime, Utc};

use rand;

use std::path::PathBuf;

use devicemapper::Bytes;

use super::super::engine::Filesystem;
use super::super::types::MaybeDbusPath;

use stratis::StratisResult;

#[derive(Debug)]
pub struct SimFilesystem {
    rand: u32,
    created: DateTime<Utc>,
    dbus_path: MaybeDbusPath,
}

impl SimFilesystem {
    pub fn new() -> SimFilesystem {
        SimFilesystem {
            rand: rand::random::<u32>(),
            created: Utc::now(),
            dbus_path: MaybeDbusPath(None),
        }
    }
}

impl Filesystem for SimFilesystem {
    fn devnode(&self) -> PathBuf {
        ["/dev/stratis", &format!("random-{}", self.rand)]
            .into_iter()
            .collect()
    }

    fn created(&self) -> DateTime<Utc> {
        self.created
    }

    fn used(&self) -> StratisResult<Bytes> {
        Ok(Bytes(12345678))
    }

    fn set_dbus_path(&mut self, path: MaybeDbusPath) -> () {
        self.dbus_path = path
    }

    fn get_dbus_path(&self) -> &MaybeDbusPath {
        &self.dbus_path
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use uuid::Uuid;

use engine::Filesystem;

use super::super::super::types::Bytes;
use super::super::engine::{HasName, HasUuid};

#[derive(Debug)]
pub struct StratFilesystem {
    pub fs_id: Uuid,
    pub name: String,
    pub thin_id: u32,
    pub mount_point: String,
    pub quota_size: Bytes,
}

impl HasName for StratFilesystem {
    fn name(&self) -> &str {
        &self.name
    }
}

impl HasUuid for StratFilesystem {
    fn uuid(&self) -> &Uuid {
        &self.fs_id
    }
}

impl Filesystem for StratFilesystem {
    fn rename(&mut self, name: &str) {
        self.name = name.to_owned();
    }

    fn mountpoint(&self) -> &String {
        &self.mount_point
    }

    fn quota(&self) -> &Bytes {
        &self.quota_size
    }
}

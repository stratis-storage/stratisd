// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use engine::Filesystem;

use super::super::super::types::Bytes;
use super::super::engine::{HasName, HasUuid};
use super::consts::DEFAULT_FILESYSTEM_QUOTA_SIZE;

#[derive(Debug)]
pub struct SimFilesystem {
    pub fs_id: Uuid,
    pub name: String,
    pub mount_point: PathBuf,
    pub quota_size: Bytes,
}

impl SimFilesystem {
    pub fn new(fs_id: Uuid,
               name: &str,
               mount_point: &Path,
               quota_size: Option<Bytes>)
               -> SimFilesystem {
        SimFilesystem {
            fs_id: fs_id,
            name: name.to_owned(),
            mount_point: mount_point.to_owned(),
            quota_size: quota_size.unwrap_or(DEFAULT_FILESYSTEM_QUOTA_SIZE),
        }
    }
}

impl Filesystem for SimFilesystem {
    fn rename(&mut self, name: &str) {
        self.name = name.to_owned();
    }

    fn mountpoint(&self) -> &Path {
        &self.mount_point
    }

    fn quota(&self) -> &Bytes {
        &self.quota_size
    }
}

impl HasName for SimFilesystem {
    fn name(&self) -> &str {
        &self.name
    }
}

impl HasUuid for SimFilesystem {
    fn uuid(&self) -> &Uuid {
        &self.fs_id
    }
}

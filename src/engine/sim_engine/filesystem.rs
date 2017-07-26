// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use super::super::engine::{HasName, HasUuid, Filesystem};
use super::super::errors::EngineResult;
use super::super::types::FilesystemUuid;

#[derive(Debug)]
pub struct SimFilesystem {
    fs_id: FilesystemUuid,
    name: String,
}

impl SimFilesystem {
    pub fn new(fs_id: FilesystemUuid, name: &str) -> SimFilesystem {
        SimFilesystem {
            fs_id: fs_id,
            name: name.to_owned(),
        }
    }

    /// Set the name of this filesystem to name.
    pub fn rename(&mut self, name: &str) {
        self.name = name.to_owned();
    }
}

impl Filesystem for SimFilesystem {
    fn devnode(&self) -> EngineResult<PathBuf> {
        Ok(PathBuf::from(format!("/dev/stratis/{}", self.name)))
    }
}

impl HasName for SimFilesystem {
    fn name(&self) -> &str {
        &self.name
    }
}

impl HasUuid for SimFilesystem {
    fn uuid(&self) -> &FilesystemUuid {
        &self.fs_id
    }
}

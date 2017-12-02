// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use super::super::engine::{Filesystem, HasName, HasUuid};
use super::super::structures::Name;
use super::super::types::FilesystemUuid;


#[derive(Debug)]
pub struct SimFilesystem {
    fs_id: FilesystemUuid,
    name: Name,
}

impl SimFilesystem {
    pub fn new(fs_id: FilesystemUuid, name: &str) -> SimFilesystem {
        SimFilesystem {
            fs_id,
            name: Name::new(name.to_owned()),
        }
    }

    /// Set the name of this filesystem to name.
    pub fn rename(&mut self, name: &str) {
        self.name = Name::new(name.to_owned());
    }
}

impl Filesystem for SimFilesystem {
    fn devnode(&self) -> PathBuf {
        ["/dev/stratis", &self.name].into_iter().collect()
    }
}

impl HasName for SimFilesystem {
    fn name(&self) -> Name {
        self.name.clone()
    }
}

impl HasUuid for SimFilesystem {
    fn uuid(&self) -> FilesystemUuid {
        self.fs_id
    }
}

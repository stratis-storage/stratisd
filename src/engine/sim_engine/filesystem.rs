// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use uuid::Uuid;

use engine::EngineResult;
use engine::Filesystem;

use super::super::engine::{HasName, HasUuid};

#[derive(Debug)]
pub struct SimFilesystem {
    fs_id: Uuid,
    name: String,
}

impl SimFilesystem {
    pub fn new(fs_id: Uuid, name: &str) -> SimFilesystem {
        SimFilesystem {
            fs_id: fs_id,
            name: name.to_owned(),
        }
    }
}

impl Filesystem for SimFilesystem {
    fn rename(&mut self, name: &str) {
        self.name = name.to_owned();
    }

    fn destroy(self) -> EngineResult<()> {
        Ok(())
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

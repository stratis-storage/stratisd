// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use uuid::Uuid;

use engine::EngineResult;
use engine::Filesystem;


#[derive(Debug, Clone,PartialEq)]
pub struct StratFilesystem {
    pub name: String,
    pub thin_id: u32,
}

impl Filesystem for StratFilesystem {
    fn get_name(&self) -> String {
        unimplemented!()
    }

    fn has_same(&self, _other: &str) -> bool {
        unimplemented!()
    }

    fn rename(&mut self, _new_name: &str) -> EngineResult<()> {
        unimplemented!()
    }

    fn add_ancestor(&mut self, _parent: Uuid) {
        unimplemented!()
    }
}

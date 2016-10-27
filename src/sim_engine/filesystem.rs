// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use engine::Filesystem;

#[derive(Debug, Clone)]
pub struct SimFilesystem {
}
impl SimFilesystem {
    pub fn new_filesystem() -> Box<SimFilesystem> {
        Box::new(SimFilesystem {})
    }
}
impl Filesystem for SimFilesystem {
    fn copy(&self) -> Box<Filesystem> {
        let filesystem_copy = SimFilesystem {};
        Box::new(filesystem_copy)
    }
}

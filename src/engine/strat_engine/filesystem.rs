// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use uuid::Uuid;

use engine::Filesystem;


#[derive(Debug)]
pub struct StratFilesystem {
    pub fs_id: Uuid,
    pub thin_id: u32,
}

impl Filesystem for StratFilesystem {}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[cfg(feature = "full_runtime")]
use rand;

use std::path::PathBuf;

use super::super::engine::Filesystem;

#[derive(Debug)]
pub struct SimFilesystem {
    rand: u32,
}

impl SimFilesystem {
    #[cfg(feature = "full_runtime")]
    pub fn new() -> SimFilesystem {
        SimFilesystem {
            rand: rand::random::<u32>(),
        }
    }
}

impl Filesystem for SimFilesystem {
    fn devnode(&self) -> PathBuf {
        ["/dev/stratis", &format!("random-{}", self.rand)]
            .into_iter()
            .collect()
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use engine::Filesystem;

#[derive(Debug, Clone)]
pub struct SimFilesystem {
    pub mount_point: String,
    pub size: u64,
}
impl SimFilesystem {
    pub fn new_filesystem(mount_point: &str, size: u64) -> SimFilesystem {
        SimFilesystem {
            mount_point: mount_point.to_owned(),
            size: size,
        }
    }
}
impl Filesystem for SimFilesystem {
    fn copy(&self) -> Box<Filesystem> {
        let filesystem_copy = SimFilesystem {
            mount_point: self.mount_point.clone(),
            size: self.size,
        };
        Box::new(filesystem_copy)
    }
    fn get_id(&self) -> String {
        self.mount_point.clone()
    }

    fn eq(&self, other: &Filesystem) -> bool {
        self.get_id() == other.get_id()
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use engine::Filesystem;

#[derive(Debug, Clone)]
pub struct SimFilesystem {
    pub name: String,
    pub mount_point: String,
    pub size: u64,
}
impl SimFilesystem {
    pub fn new_filesystem(filesystem_name: &str,
                          mount_point: &str,
                          size: u64)
                          -> Box<SimFilesystem> {
        Box::new(SimFilesystem {
            name: filesystem_name.to_owned(),
            mount_point: mount_point.to_owned(),
            size: size,
        })
    }
}
impl Filesystem for SimFilesystem {
    fn copy(&self) -> Box<Filesystem> {
        let filesystem_copy = SimFilesystem {
            name: self.name.clone(),
            mount_point: self.mount_point.clone(),
            size: self.size,
        };
        Box::new(filesystem_copy)
    }
}

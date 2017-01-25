// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use uuid::Uuid;

use engine::Filesystem;

use super::consts::DEFAULT_FILESYSTEM_QUOTA_SIZE;

#[derive(Debug)]
pub struct SimFilesystem {
    pub fs_id: Uuid,
    pub mount_point: String,
    pub quota_size: u64,
    pub nearest_ancestor: Option<Uuid>,
}

impl SimFilesystem {
    pub fn new_filesystem(fs_id: Uuid,
                          mount_point: &str,
                          quota_size: Option<u64>)
                          -> SimFilesystem {
        SimFilesystem {
            fs_id: fs_id,
            mount_point: mount_point.to_owned(),
            quota_size: quota_size.unwrap_or(DEFAULT_FILESYSTEM_QUOTA_SIZE),
            nearest_ancestor: None,
        }
    }
}
impl Filesystem for SimFilesystem {}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use engine::Filesystem;
use engine::EngineResult;

use uuid::Uuid;

use super::consts::DEFAULT_FILESYSTEM_SIZE;

#[derive(Debug, Clone)]
pub struct SimFilesystem {
    pub uuid: Uuid,
    pub name: String,
    pub mount_point: String,
    pub size: u64,
}
impl SimFilesystem {
    pub fn new_filesystem(name: &str, mount_point: &str, size: Option<u64>) -> SimFilesystem {
        SimFilesystem {
            name: name.to_owned(),
            uuid: Uuid::new_v4(),
            mount_point: mount_point.to_owned(),
            size: size.unwrap_or(DEFAULT_FILESYSTEM_SIZE),
        }
    }
}
impl Filesystem for SimFilesystem {
    fn get_id(&self) -> Uuid {
        self.uuid.clone()
    }

    fn eq(&self, other: &Filesystem) -> bool {
        self.get_id() == other.get_id()
    }

    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn has_same(&self, other: &str) -> bool {
        self.get_name() == other
    }

    fn rename(&mut self, new_name: &str) -> EngineResult<()> {
        self.name = String::from(new_name);
        Ok(())
    }
}

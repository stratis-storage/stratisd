// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use zbus::zvariant::OwnedObjectPath;

use crate::engine::{DevUuid, FilesystemUuid, PoolUuid};

mod manager_3_0;
mod manager_3_8;
mod manager_3_9;

pub use manager_3_9::ManagerR9;

#[derive(Default)]
pub struct Manager {
    pub pools: HashMap<PoolUuid, OwnedObjectPath>,
    pub filesystems: HashMap<FilesystemUuid, OwnedObjectPath>,
    pub blockdevs: HashMap<DevUuid, OwnedObjectPath>,
}

impl Manager {
    pub fn add_pool(&mut self, uuid: PoolUuid, path: OwnedObjectPath) {
        self.pools.insert(uuid, path);
    }

    pub fn add_filesystem(&mut self, uuid: FilesystemUuid, path: OwnedObjectPath) {
        self.filesystems.insert(uuid, path);
    }

    pub fn add_blockdev(&mut self, uuid: DevUuid, path: OwnedObjectPath) {
        self.blockdevs.insert(uuid, path);
    }

    pub fn remove_pool(&mut self, uuid: &PoolUuid) {
        self.pools.remove(uuid);
    }

    pub fn remove_filesystem(&mut self, uuid: &FilesystemUuid) {
        self.filesystems.remove(uuid);
    }

    pub fn remove_blockdev(&mut self, uuid: &DevUuid) {
        self.blockdevs.remove(uuid);
    }
}

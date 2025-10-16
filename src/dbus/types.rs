// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use zbus::zvariant::{Signature, Type};

use crate::engine::{FilesystemUuid, KeyDescription, PoolDiff, PoolUuid, StratFilesystemDiff};

impl Type for KeyDescription {
    const SIGNATURE: &Signature = &Signature::Str;
}

pub enum DbusAction {}

impl DbusAction {
    pub fn from_fs_diffs(_: HashMap<FilesystemUuid, StratFilesystemDiff>) -> Vec<Self> {
        vec![]
    }

    pub fn from_pool_diffs(_: HashMap<PoolUuid, PoolDiff>) -> Vec<Self> {
        vec![]
    }
}

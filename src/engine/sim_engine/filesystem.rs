// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use chrono::{DateTime, Utc};

use std::path::PathBuf;

use devicemapper::Bytes;

use crate::{engine::Filesystem, stratis::StratisResult};

#[derive(Debug)]
pub struct SimFilesystem {
    rand: u32,
    created: DateTime<Utc>,
}

impl SimFilesystem {
    pub fn new() -> SimFilesystem {
        SimFilesystem {
            rand: rand::random::<u32>(),
            created: Utc::now(),
        }
    }
}

impl Filesystem for SimFilesystem {
    fn devnode(&self) -> PathBuf {
        ["/stratis", &format!("random-{}", self.rand)]
            .iter()
            .collect()
    }

    fn created(&self) -> DateTime<Utc> {
        self.created
    }

    fn path_to_mount_filesystem(&self, pool_name: &str, fs_name: &str) -> PathBuf {
        vec!["/somepath", pool_name, fs_name].iter().collect()
    }

    fn used(&self) -> StratisResult<Bytes> {
        Ok(Bytes(12_345_678))
    }
}

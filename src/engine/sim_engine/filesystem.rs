// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use devicemapper::{Bytes, Sectors};

use crate::{engine::Filesystem, stratis::StratisResult};

#[derive(Debug)]
pub struct SimFilesystem {
    rand: u32,
    created: DateTime<Utc>,
    size: Sectors,
}

impl SimFilesystem {
    pub fn new(size: Sectors) -> SimFilesystem {
        SimFilesystem {
            rand: rand::random::<u32>(),
            created: Utc::now(),
            size,
        }
    }

    pub fn size(&self) -> Sectors {
        self.size
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
        Ok((self.size / 2u64).bytes())
    }
}

impl<'a> Into<Value> for &'a SimFilesystem {
    fn into(self) -> Value {
        let mut json = Map::new();
        json.insert("size".to_string(), Value::from(self.size().to_string()));
        json.insert(
            "used".to_string(),
            Value::from(
                self.used()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|_| "Unavailable".to_string()),
            ),
        );
        Value::from(json)
    }
}

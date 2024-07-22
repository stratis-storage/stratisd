// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use devicemapper::{Bytes, Sectors, IEC};

use crate::engine::{
    engine::BlockDev,
    shared::now_to_timestamp,
    types::{DevUuid, StratSigblockVersion},
};

#[derive(Debug)]
/// A simulated device.
pub struct SimDev {
    devnode: PathBuf,
    user_info: Option<String>,
    hardware_info: Option<String>,
    initialization_time: DateTime<Utc>,
}

impl SimDev {
    /// Access a structure containing the simulated device path
    pub fn devnode(&self) -> &Path {
        &self.devnode
    }
}

impl BlockDev for SimDev {
    fn devnode(&self) -> &Path {
        self.devnode()
    }

    fn metadata_path(&self) -> &Path {
        self.devnode()
    }

    fn user_info(&self) -> Option<&str> {
        self.user_info.as_deref()
    }

    fn hardware_info(&self) -> Option<&str> {
        self.hardware_info.as_deref()
    }

    fn initialization_time(&self) -> DateTime<Utc> {
        self.initialization_time
    }

    fn size(&self) -> Sectors {
        Bytes::from(IEC::Gi).sectors()
    }

    fn new_size(&self) -> Option<Sectors> {
        None
    }

    fn metadata_version(&self) -> StratSigblockVersion {
        StratSigblockVersion::V2
    }
}

impl SimDev {
    /// Generates a new device from any devnode.
    pub fn new(devnode: &Path) -> (DevUuid, SimDev) {
        (
            DevUuid::new_v4(),
            SimDev {
                devnode: devnode.to_owned(),
                user_info: None,
                hardware_info: None,
                initialization_time: now_to_timestamp(),
            },
        )
    }

    /// Set the user info on this blockdev.
    /// The user_info may be None, which unsets user info.
    /// Returns true if the user info was changed, otherwise false.
    pub fn set_user_info(&mut self, user_info: Option<&str>) -> bool {
        set_blockdev_user_info!(self; user_info)
    }
}

impl<'a> Into<Value> for &'a SimDev {
    fn into(self) -> Value {
        let mut json = Map::new();
        json.insert(
            "path".to_string(),
            Value::from(self.devnode.display().to_string()),
        );
        json.insert("size".to_string(), Value::from(self.size().to_string()));
        Value::from(json)
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Map, Value};

use devicemapper::{Bytes, Sectors, IEC};

use crate::engine::{
    engine::BlockDev,
    types::{DevUuid, EncryptionInfo, KeyDescription},
};

#[derive(Debug)]
/// A simulated device.
pub struct SimDev {
    devnode: PathBuf,
    user_info: Option<String>,
    hardware_info: Option<String>,
    initialization_time: u64,
    encryption_info: Option<EncryptionInfo>,
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
        Utc.timestamp(self.initialization_time as i64, 0)
    }

    fn size(&self) -> Sectors {
        Bytes::from(IEC::Gi).sectors()
    }

    fn is_encrypted(&self) -> bool {
        self.encryption_info.is_some()
    }

    fn allocated(&self) -> Sectors {
        Bytes::from(IEC::Mi).sectors()
    }

    fn real_size(&self) -> Sectors {
        2usize * Bytes::from(IEC::Gi).sectors()
    }
}

impl SimDev {
    /// Generates a new device from any devnode.
    pub fn new(devnode: &Path, encryption_info: Option<&EncryptionInfo>) -> (DevUuid, SimDev) {
        (
            DevUuid::new_v4(),
            SimDev {
                devnode: devnode.to_owned(),
                user_info: None,
                hardware_info: None,
                initialization_time: Utc::now().timestamp() as u64,
                encryption_info: encryption_info.cloned(),
            },
        )
    }

    /// Set the user info on this blockdev.
    /// The user_info may be None, which unsets user info.
    /// Returns true if the user info was changed, otherwise false.
    pub fn set_user_info(&mut self, user_info: Option<&str>) -> bool {
        set_blockdev_user_info!(self; user_info)
    }

    /// Set the clevis info for a block device.
    pub fn set_clevis_info(&mut self, pin: &str, config: &Value) {
        self.encryption_info = self
            .encryption_info
            .take()
            .map(|ei| ei.set_clevis_info((pin.to_owned(), config.clone())));
    }

    /// Unset the clevis info for a block device.
    pub fn unset_clevis_info(&mut self) {
        self.encryption_info = self.encryption_info.take().map(|ei| ei.unset_clevis_info());
    }

    /// Set the key description for a block device.
    pub fn set_key_desc(&mut self, key_desc: &KeyDescription) {
        self.encryption_info = self
            .encryption_info
            .take()
            .map(|ei| ei.set_key_desc(key_desc.clone()))
    }

    /// Unset the key description for a block device.
    pub fn unset_key_desc(&mut self) {
        self.encryption_info = self.encryption_info.take().map(|ei| ei.unset_key_desc())
    }

    /// Get encryption information for this block device.
    pub fn encryption_info(&self) -> Option<&EncryptionInfo> {
        self.encryption_info.as_ref()
    }
}

impl<'a> Into<Value> for &'a SimDev {
    fn into(self) -> Value {
        let mut json = Map::new();
        json.insert(
            "path".to_string(),
            Value::from(self.devnode.display().to_string()),
        );
        if let Some(EncryptionInfo::Both(kd, (pin, config))) = self.encryption_info.as_ref() {
            json.insert(
                "key_description".to_string(),
                Value::from(kd.as_application_str()),
            );
            json.insert("clevis_pin".to_string(), Value::from(pin.to_owned()));
            json.insert("clevis_config".to_string(), config.to_owned());
        } else if let Some(EncryptionInfo::KeyDesc(kd)) = self.encryption_info.as_ref() {
            json.insert(
                "key_description".to_string(),
                Value::from(kd.as_application_str()),
            );
        } else if let Some(EncryptionInfo::ClevisInfo((pin, config))) =
            self.encryption_info.as_ref()
        {
            json.insert("clevis_pin".to_string(), Value::from(pin.to_owned()));
            json.insert("clevis_config".to_string(), config.to_owned());
        }
        Value::from(json)
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Map, Value};
use uuid::Uuid;

use devicemapper::{Bytes, Sectors, IEC};

use crate::engine::{
    engine::BlockDev,
    sim_engine::randomization::Randomizer,
    types::{BlockDevPath, EncryptionInfo, MaybeDbusPath},
};

#[derive(Debug)]
/// A simulated device.
pub struct SimDev {
    devnode: BlockDevPath,
    rdm: Randomizer,
    user_info: Option<String>,
    hardware_info: Option<String>,
    initialization_time: u64,
    dbus_path: MaybeDbusPath,
    encryption_info: Option<EncryptionInfo>,
}

impl SimDev {
    /// Access a structure containing the simulated device path
    pub fn devnode(&self) -> &BlockDevPath {
        &self.devnode
    }
}

impl BlockDev for SimDev {
    fn devnode(&self) -> &BlockDevPath {
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
        Bytes(IEC::Gi).sectors()
    }

    fn set_dbus_path(&mut self, path: MaybeDbusPath) {
        self.dbus_path = path
    }

    fn get_dbus_path(&self) -> &MaybeDbusPath {
        &self.dbus_path
    }

    fn is_encrypted(&self) -> bool {
        self.encryption_info.is_some()
    }
}

impl SimDev {
    /// Generates a new device from any devnode.
    pub fn new(devnode: &Path, encryption_info: Option<&EncryptionInfo>) -> (Uuid, SimDev) {
        (
            Uuid::new_v4(),
            SimDev {
                devnode: BlockDevPath::physical_device_path(devnode),
                rdm: Randomizer::default(),
                user_info: None,
                hardware_info: None,
                initialization_time: Utc::now().timestamp() as u64,
                dbus_path: MaybeDbusPath(None),
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
    pub fn set_clevis_info(&mut self, pin: String, config: Value) {
        if let Some(ref mut info) = self.encryption_info {
            info.clevis_info = Some((pin, config));
        }
    }

    /// Unset the clevis info for a block device.
    pub fn unset_clevis_info(&mut self) {
        if let Some(ref mut info) = self.encryption_info {
            info.clevis_info = None;
        }
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
            Value::from(self.devnode.physical_path().display().to_string()),
        );
        if let Some(EncryptionInfo {
            ref key_description,
            ref clevis_info,
        }) = self.encryption_info
        {
            json.insert(
                "key_description".to_string(),
                Value::from(key_description.as_application_str()),
            );
            if let Some((ref pin, ref config)) = clevis_info {
                json.insert("clevis_pin".to_string(), Value::from(pin.to_owned()));
                json.insert("clevis_config".to_string(), config.to_owned());
            }
        }
        Value::from(json)
    }
}

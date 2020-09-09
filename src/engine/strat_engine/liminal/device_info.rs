// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Types representing known information about devices

use std::fmt;

use serde_json::Value;

use crate::engine::{
    strat_engine::{
        liminal::identify::{LuksInfo, OwnedDeviceInfo, StratisInfo},
        metadata::StratisIdentifiers,
    },
    types::KeyDescription,
};

/// Info for a discovered Luks Device belonging to Stratis.
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct LLuksInfo {
    /// Generic information + Stratis identifiers
    pub ids: StratisInfo,
    pub key_description: KeyDescription,
}

impl fmt::Display for LLuksInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}, key description: \"{}\"",
            self.ids,
            self.key_description.as_application_str()
        )
    }
}

impl From<LuksInfo> for LLuksInfo {
    fn from(info: LuksInfo) -> LLuksInfo {
        LLuksInfo {
            ids: info.info,
            key_description: info.key_description,
        }
    }
}

impl<'a> Into<Value> for &'a LLuksInfo {
    // Precondition: (&StratisInfo).into() pattern matches Value::Object()
    fn into(self) -> Value {
        let mut json = json!({
            "key_description": Value::from(self.key_description.as_application_str())
        });
        if let Value::Object(ref mut map) = json {
            map.extend(
                if let Value::Object(map) = <&StratisInfo as Into<Value>>::into(&self.ids) {
                    map.into_iter()
                } else {
                    unreachable!("StratisInfo conversion returns a JSON object");
                },
            );
        } else {
            unreachable!("json!() always creates a JSON object");
        };
        json
    }
}

/// Info for a Stratis device.
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct LStratisInfo {
    /// Generic information + Stratis identifiers
    pub ids: StratisInfo,
    /// Luks information. The information will be set if this is a Stratis
    /// device which is an activated encrypted device, and will be
    /// information about that physical device.
    pub luks: Option<LLuksInfo>,
}

impl fmt::Display for LStratisInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(info) = &self.luks {
            write!(
                f,
                "logical device with {} and physical device with {}",
                self.ids, info
            )
        } else {
            write!(f, "{}", self.ids)
        }
    }
}

impl From<StratisInfo> for LStratisInfo {
    fn from(info: StratisInfo) -> LStratisInfo {
        LStratisInfo {
            ids: info,
            luks: None,
        }
    }
}

impl<'a> Into<Value> for &'a LStratisInfo {
    // Precondition: (&StratisInfo).into() pattern matches Value::Object()
    // Precondition: (&LLuksInfo).into() pattern matches Value::Object()
    fn into(self) -> Value {
        let mut json = self
            .luks
            .as_ref()
            .map(|luks| json!({ "luks": <&LLuksInfo as Into<Value>>::into(luks) }))
            .unwrap_or_else(|| json!({}));
        if let Value::Object(ref mut map) = json {
            map.extend(
                if let Value::Object(map) = <&StratisInfo as Into<Value>>::into(&self.ids) {
                    map.into_iter()
                } else {
                    unreachable!("StratisInfo conversion returns a JSON object");
                },
            );
        } else {
            unreachable!("json!() always creates a JSON object");
        };
        json
    }
}

impl LStratisInfo {
    #[allow(dead_code)]
    fn invariant(&self) {
        assert!(match &self.luks {
            None => true,
            Some(luks) =>
                luks.ids.identifiers == self.ids.identifiers
                    && luks.ids.devnode != self.ids.devnode
                    && luks.ids.device_number != self.ids.device_number,
        });
    }
}

/// A unifying Info struct for Stratis or Luks devices
#[derive(Debug, Eq, Hash, PartialEq)]
pub enum LInfo {
    /// A Stratis device, which may be an encrypted device
    Stratis(LStratisInfo),
    /// A LUKS device
    Luks(LLuksInfo),
}

impl fmt::Display for LInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            LInfo::Stratis(info) => write!(f, "Stratis device with {}", info),
            LInfo::Luks(info) => write!(f, "LUKS device belonging to Stratis with {}", info),
        }
    }
}

impl From<OwnedDeviceInfo> for LInfo {
    fn from(info: OwnedDeviceInfo) -> LInfo {
        match info {
            OwnedDeviceInfo::Luks(info) => LInfo::Luks(info.into()),
            OwnedDeviceInfo::Stratis(info) => LInfo::Stratis(info.into()),
        }
    }
}

impl<'a> Into<Value> for &'a LInfo {
    // Precondition: (&LStratisInfo).into() pattern matches Value::Object()
    // Precondition: (&LLuksInfo).into() pattern matches Value::Object()
    fn into(self) -> Value {
        match self {
            LInfo::Stratis(info) => info.into(),
            LInfo::Luks(info) => info.into(),
        }
    }
}

impl LInfo {
    pub fn stratis_identifiers(&self) -> StratisIdentifiers {
        match self {
            LInfo::Luks(info) => info.ids.identifiers,
            LInfo::Stratis(info) => info.ids.identifiers,
        }
    }

    pub fn key_desc(&self) -> Option<&KeyDescription> {
        match self {
            LInfo::Luks(info) => Some(&info.key_description),
            LInfo::Stratis(info) => info.luks.as_ref().map(|i| &i.key_description),
        }
    }

    /// Combine two devices which have identical pool and device UUIDs.
    /// The first argument is the existing information, the second is the
    /// information about the removed device, where "removed" means there
    /// was a udev "remove" event and this info has been found out about the
    /// device attached to the event.
    pub fn update_on_remove(info_1: LInfo, info_2: LInfo) -> Option<LInfo> {
        match (info_1, info_2) {
            (luks_info @ LInfo::Luks(_), LInfo::Stratis(_)) => Some(luks_info),
            (LInfo::Stratis(strat_info), LInfo::Luks(luks_info)) => {
                if let Some(luks) = &strat_info.luks {
                    if luks.ids.device_number != luks_info.ids.device_number {
                        warn!("Received udev remove event on a device with {} that stratisd does not know about; retaining logical device with {} among the set of devices known to belong to pool with UUID {}",
                                luks_info,
                                strat_info,
                                strat_info.ids.identifiers.pool_uuid);
                    } else {
                        warn!("Received udev remove event on a device with {} that appeared to belong to Stratis, but the logical device information is still present; retaining the logical device with the original encryption information",
                                  luks_info);
                    }
                }
                Some(LInfo::Stratis(strat_info))
            }
            (LInfo::Stratis(info_1), LInfo::Stratis(info_2)) => {
                if info_1.ids.device_number != info_2.ids.device_number {
                    warn!("Received udev remove event on a device with {} that stratisd does not know about; retaining duplicate device {} among the set of devices known to belong to pool with UUID {}",
                              info_2,
                              info_1,
                              info_1.ids.identifiers.pool_uuid);
                    Some(LInfo::Stratis(info_1))
                } else {
                    info_1.luks.map(LInfo::Luks)
                }
            }
            (LInfo::Luks(info_1), LInfo::Luks(info_2)) => {
                if info_1.ids.device_number != info_2.ids.device_number {
                    warn!("Received udev remove event on a device with {} that stratisd does not know about; retaining duplicate device {} among the set of devices known to belong to pool with UUID {}",
                              info_2,
                              info_1,
                              info_1.ids.identifiers.pool_uuid);
                    Some(LInfo::Luks(info_1))
                } else {
                    None
                }
            }
        }
    }

    // Combine two devices which have identical pool and device UUIDs.
    // The first argument is the older information, the second the newer.
    // Allow the newer information to supplant the older.
    // Precondition: the newer information must always represent a single
    // device, so the luks field of a newly discovered Stratis device
    // must always be None.
    pub fn update(info_1: LInfo, info_2: LInfo) -> Result<LInfo, (String, LInfo, LInfo)> {
        // Returns true if the information found via udev for two devices is
        // compatible, otherwise false.
        // Precondition: Stratis identifiers of devices are the same
        fn luks_luks_compatible(info_1: &LLuksInfo, info_2: &LLuksInfo) -> bool {
            assert_eq!(info_1.ids.identifiers, info_2.ids.identifiers);
            info_1.ids.device_number == info_2.ids.device_number
                && info_1.key_description == info_2.key_description
        }

        // Returns true if the information found via udev for two devices is
        // compatible, otherwise false.
        // Precondition: Stratis identifiers of devices are the same
        fn stratis_stratis_compatible(info_1: &LStratisInfo, info_2: &LStratisInfo) -> bool {
            assert_eq!(info_1.ids.identifiers, info_2.ids.identifiers);
            info_1.ids.device_number == info_2.ids.device_number
                && match (info_1.luks.as_ref(), info_2.luks.as_ref()) {
                    (Some(luks_1), Some(luks_2)) => luks_luks_compatible(luks_1, luks_2),
                    _ => true,
                }
        }
        match (info_1, info_2) {
            (LInfo::Luks(luks_info), LInfo::Stratis(strat_info)) => {
                assert_eq!(strat_info.luks, None);
                Ok(LInfo::Stratis(LStratisInfo {
                    ids: strat_info.ids,
                    luks: Some(luks_info),
                }))
            }
            (LInfo::Stratis(strat_info), LInfo::Luks(luks_info)) => {
                if let Some(luks) = strat_info.luks.as_ref() {
                    if !luks_luks_compatible(luks, &luks_info) {
                        let (info_1, info_2) = (LInfo::Stratis(strat_info), LInfo::Luks(luks_info));
                        let err_msg = format!(
                                "Information about previously discovered device {} incompatible with information about newly discovered device {}",
                                info_1,
                                info_2);

                        return Err((err_msg, info_1, info_2));
                    }
                }
                Ok(LInfo::Stratis(LStratisInfo {
                    ids: strat_info.ids,
                    luks: Some(luks_info),
                }))
            }
            (LInfo::Luks(luks_info_1), LInfo::Luks(luks_info_2)) => {
                if !luks_luks_compatible(&luks_info_1, &luks_info_2) {
                    let (info_1, info_2) = (LInfo::Luks(luks_info_1), LInfo::Luks(luks_info_2));
                    let err_msg = format!(
                            "Information about previously discovered device {} incompatible with information about newly discovered device {}",
                            info_1,
                            info_2);
                    Err((err_msg, info_1, info_2))
                } else {
                    Ok(LInfo::Luks(luks_info_2))
                }
            }
            (LInfo::Stratis(strat_info_1), LInfo::Stratis(strat_info_2)) => {
                if !stratis_stratis_compatible(&strat_info_1, &strat_info_2) {
                    let (info_1, info_2) =
                        (LInfo::Stratis(strat_info_1), LInfo::Stratis(strat_info_2));
                    let err_msg = format!(
                            "Information about previously discovered device {} incompatible with information about newly discovered device {}",
                            info_1,
                            info_2);
                    Err((err_msg, info_1, info_2))
                } else {
                    Ok(LInfo::Stratis(LStratisInfo {
                        ids: strat_info_2.ids,
                        luks: strat_info_2.luks.or(strat_info_1.luks),
                    }))
                }
            }
        }
    }
}

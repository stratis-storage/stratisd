// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Types representing known information about devices

use std::fmt;

use serde_json::Value;

use crate::engine::{
    strat_engine::backstore::{DeviceInfo, LuksInfo, StratisIdentifiers, StratisInfo},
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

impl From<DeviceInfo> for LInfo {
    fn from(info: DeviceInfo) -> LInfo {
        match info {
            DeviceInfo::Luks(info) => LInfo::Luks(info.into()),
            DeviceInfo::Stratis(info) => LInfo::Stratis(info.into()),
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
}

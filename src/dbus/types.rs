// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use zbus::zvariant::{signature::Child, Basic, Dict, Signature, Type, Value};

use crate::{
    dbus::util::result_option_to_tuple,
    engine::{LockedPoolsInfo, PoolUuid},
};

pub type FilesystemSpec<'a> = Vec<(&'a str, (bool, &'a str), (bool, &'a str))>;

#[derive(Clone, Copy, Debug)]
#[allow(non_camel_case_types)]
pub enum DbusErrorEnum {
    OK = 0,
    ERROR = 1,
}

impl Type for LockedPoolsInfo {
    const SIGNATURE: &Signature = &Signature::Dict {
        key: Child::Static {
            child: &Signature::Str,
        },
        value: Child::Static {
            child: &Signature::Dict {
                key: Child::Static {
                    child: &Signature::Str,
                },
                value: Child::Static {
                    child: &Signature::Variant,
                },
            },
        },
    };
}

impl<'a> From<LockedPoolsInfo> for Value<'a> {
    fn from(infos: LockedPoolsInfo) -> Self {
        let mut top_level_dict = Dict::new(
            &Signature::Str,
            &Signature::Dict {
                key: Child::Static {
                    child: &Signature::Str,
                },
                value: Child::Static {
                    child: &Signature::Variant,
                },
            },
        );
        for (uuid, info) in infos.locked.iter() {
            let mut dict = Dict::new(&Signature::Str, &Signature::Variant);
            if let Err(e) = dict.add(
                "key_description",
                result_option_to_tuple(
                    info.info
                        .key_description()
                        .map(|opt| opt.map(|kd| kd.as_application_str().to_owned())),
                    String::new(),
                ),
            ) {
                warn!("Failed to convert locked pool information to D-Bus format: {e}");
            };
            if let Err(e) = dict.add(
                "clevis_info",
                result_option_to_tuple(
                    info.info
                        .clevis_info()
                        .map(|opt| opt.map(|(pin, value)| (pin.to_owned(), value.to_string()))),
                    (String::new(), String::new()),
                ),
            ) {
                warn!("Failed to convert locked pool information to D-Bus format: {e}");
            };
            if let Err(e) = dict.add(
                "devs",
                info.devices
                    .iter()
                    .map(|d| {
                        let mut map = HashMap::new();
                        map.insert(
                            "devnode".to_string(),
                            Value::from(d.devnode.display().to_string()),
                        );
                        map.insert("uuid".to_string(), Value::from(d.uuid));
                        map
                    })
                    .collect::<Vec<_>>(),
            ) {
                warn!("Failed to convert locked pool information to D-Bus format: {e}");
            };
            if let Some(name) = infos.uuid_to_name.get(uuid) {
                if let Err(e) = dict.add("name", Value::from(name.clone())) {
                    warn!("Failed to convert locked pool information to D-Bus format: {e}");
                };
            }
            if let Err(e) = top_level_dict.add(*uuid, Value::Dict(dict)) {
                warn!("Failed to convert locked pool information to D-Bus format: {e}");
            }
        }
        Value::from(top_level_dict)
    }
}

impl Basic for PoolUuid {
    const SIGNATURE_CHAR: char = 's';
    const SIGNATURE_STR: &str = "s";
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use zbus::zvariant::{signature::Child, Basic, Dict, Signature, Type, Value};

use crate::{
    dbus::util::result_option_to_tuple,
    engine::{ActionAvailability, LockedPoolsInfo, PoolUuid, StoppedPoolsInfo},
};

pub type FilesystemSpec<'a> = Vec<(&'a str, (bool, &'a str), (bool, &'a str))>;

/// Wrapper type indicating that the return value is being returned from Manager.r2
pub struct ManagerR2<T> {
    pub inner: T,
}

/// Wrapper type indicating that the return value is being returned from Manager.r8
pub struct ManagerR8<T> {
    pub inner: T,
}

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

fn stopped_pools_to_value<'b>(infos: &StoppedPoolsInfo, metadata: bool) -> Dict<'b, 'b> {
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
    for (uuid, info) in infos
        .stopped
        .iter()
        .chain(infos.partially_constructed.iter())
    {
        let mut dict = Dict::new(&Signature::Str, &Signature::Variant);
        if let Some(enc_info) = info.info.as_ref() {
            if let Err(e) = dict.add(
                "key_description",
                result_option_to_tuple(
                    enc_info
                        .key_description()
                        .map(|opt| opt.map(|kd| kd.as_application_str().to_owned())),
                    String::new(),
                ),
            ) {
                warn!("Failed to convert stopped pool information to D-Bus format: {e}");
            };
            if let Err(e) = dict.add(
                "clevis_info",
                result_option_to_tuple(
                    enc_info
                        .clevis_info()
                        .map(|opt| opt.map(|(pin, value)| (pin.to_owned(), value.to_string()))),
                    (String::new(), String::new()),
                ),
            ) {
                warn!("Failed to convert stopped pool information to D-Bus format: {e}");
            };
        }
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
            warn!("Failed to convert stopped pool information to D-Bus format: {e}");
        };
        if let Some(name) = infos.uuid_to_name.get(uuid) {
            if let Err(e) = dict.add("name", Value::from(name.clone())) {
                warn!("Failed to convert stopped pool information to D-Bus format: {e}");
            };
        }
        if metadata {
            if let Err(e) = dict.add(
                "metadata_version".to_string(),
                match info.metadata_version {
                    Some(m) => Value::from((true, m as u64)),
                    None => Value::from((false, 0)),
                },
            ) {
                warn!("Failed to convert stopped pool information to D-Bus format: {e}");
            };
            if let Err(e) = dict.add(
                "features".to_string(),
                match info.features {
                    Some(ref f) => {
                        let mut feat = HashMap::new();
                        if f.encryption {
                            feat.insert("encryption".to_string(), true);
                        }
                        if f.key_description_enabled {
                            feat.insert("key_description_present".to_string(), true);
                        }
                        if f.clevis_enabled {
                            feat.insert("clevis_present".to_string(), true);
                        }
                        Value::from((true, feat))
                    }
                    None => Value::from((false, HashMap::<String, bool>::new())),
                },
            ) {
                warn!("Failed to convert stopped pool information to D-Bus format: {e}");
            };
        }
        if let Err(e) = top_level_dict.add(*uuid, Value::Dict(dict)) {
            warn!("Failed to convert stopped pool information to D-Bus format: {e}");
        }
    }

    top_level_dict
}

impl Type for ManagerR2<StoppedPoolsInfo> {
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

impl<'a> From<ManagerR2<StoppedPoolsInfo>> for Value<'a> {
    fn from(wrapper: ManagerR2<StoppedPoolsInfo>) -> Self {
        let infos = wrapper.inner;

        let top_level_dict = stopped_pools_to_value(&infos, false);

        Value::from(top_level_dict)
    }
}

impl Type for ManagerR8<StoppedPoolsInfo> {
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

impl<'a> From<ManagerR8<StoppedPoolsInfo>> for Value<'a> {
    fn from(wrapper: ManagerR8<StoppedPoolsInfo>) -> Self {
        let infos = wrapper.inner;

        let top_level_dict = stopped_pools_to_value(&infos, true);

        Value::from(top_level_dict)
    }
}

impl Basic for PoolUuid {
    const SIGNATURE_CHAR: char = 's';
    const SIGNATURE_STR: &str = "s";
}

impl Type for ActionAvailability {
    const SIGNATURE: &Signature = &Signature::Str;
}

impl<'a> From<ActionAvailability> for Value<'a> {
    fn from(aa: ActionAvailability) -> Self {
        Value::from(aa.to_string())
    }
}

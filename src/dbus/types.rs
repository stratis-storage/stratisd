// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use zbus::zvariant::{signature::Child, Basic, Signature, Type, Value};

use crate::{
    dbus::util::result_option_to_tuple,
    engine::{LockedPoolsInfo, PoolUuid, StoppedPoolsInfo},
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
        let mut top_level_dict = HashMap::new();
        for (uuid, info) in infos.locked.iter() {
            let mut dict = HashMap::new();
            dict.insert(
                "key_description".to_string(),
                Value::from(result_option_to_tuple(
                    info.info
                        .key_description()
                        .map(|opt| opt.map(|kd| kd.as_application_str().to_owned())),
                    String::new(),
                )),
            );
            dict.insert(
                "clevis_info".to_string(),
                Value::from(result_option_to_tuple(
                    info.info
                        .clevis_info()
                        .map(|opt| opt.map(|(pin, value)| (pin.to_owned(), value.to_string()))),
                    (String::new(), String::new()),
                )),
            );
            dict.insert(
                "devs".to_string(),
                Value::from(
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
                ),
            );
            if let Some(name) = infos.uuid_to_name.get(uuid) {
                dict.insert("name".to_string(), Value::from(name.clone()));
            }
            top_level_dict.insert(*uuid, dict);
        }
        Value::from(top_level_dict)
    }
}

fn stopped_pools_to_value<'b>(
    infos: &StoppedPoolsInfo,
    metadata: bool,
) -> HashMap<PoolUuid, HashMap<String, Value<'b>>> {
    let mut top_level_dict = HashMap::new();
    for (uuid, info) in infos
        .stopped
        .iter()
        .chain(infos.partially_constructed.iter())
    {
        let mut dict = HashMap::new();
        if let Some(enc_info) = info.info.as_ref() {
            dict.insert(
                "key_description".to_string(),
                Value::from(result_option_to_tuple(
                    enc_info
                        .key_description()
                        .map(|opt| opt.map(|kd| kd.as_application_str().to_owned())),
                    String::new(),
                )),
            );
            dict.insert(
                "clevis_info".to_string(),
                Value::from(result_option_to_tuple(
                    enc_info
                        .clevis_info()
                        .map(|opt| opt.map(|(pin, value)| (pin.to_owned(), value.to_string()))),
                    (String::new(), String::new()),
                )),
            );
        }
        dict.insert(
            "devs".to_string(),
            Value::from(
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
            ),
        );
        if let Some(name) = infos.uuid_to_name.get(uuid) {
            dict.insert("name".to_string(), Value::from(name.clone()));
        }
        if metadata {
            dict.insert(
                "metadata_version".to_string(),
                match info.metadata_version {
                    Some(m) => Value::from((true, m as u64)),
                    None => Value::from((false, 0)),
                },
            );
            dict.insert(
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
            );
        }
        top_level_dict.insert(*uuid, dict);
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

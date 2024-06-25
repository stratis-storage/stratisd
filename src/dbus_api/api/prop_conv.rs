// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use dbus::arg::{RefArg, Variant};

use crate::{
    dbus_api::util::result_option_to_tuple,
    engine::{LockedPoolsInfo, StoppedPoolsInfo},
};

/// D-Bus representation of locked pools.
pub type StoppedOrLockedPools = HashMap<String, HashMap<String, Variant<Box<dyn RefArg>>>>;

/// Convert a locked pool data structure to a property format.
pub fn locked_pools_to_prop(pools: &LockedPoolsInfo) -> StoppedOrLockedPools {
    pools
        .locked
        .iter()
        .map(|(u, locked)| {
            let uuid = uuid_to_string!(u);
            let mut info = HashMap::new();
            info.insert(
                "key_description".to_string(),
                Variant(Box::new(result_option_to_tuple(
                    locked
                        .info
                        .key_description()
                        .map(|opt| opt.map(|kd| kd.as_application_str().to_string())),
                    String::new(),
                )) as Box<dyn RefArg>),
            );
            info.insert(
                "clevis_info".to_string(),
                Variant(Box::new(result_option_to_tuple(
                    locked
                        .info
                        .clevis_info()
                        .map(|opt| opt.map(|(pin, cfg)| (pin.to_owned(), cfg.to_string()))),
                    (String::new(), String::new()),
                )) as Box<dyn RefArg>),
            );
            info.insert(
                "devs".to_string(),
                Variant(Box::new(
                    locked
                        .devices
                        .iter()
                        .map(|d| {
                            let mut map = HashMap::new();
                            map.insert("devnode".to_string(), d.devnode.display().to_string());
                            map.insert("uuid".to_string(), uuid_to_string!(d.uuid));
                            map
                        })
                        .collect::<Vec<_>>(),
                )),
            );
            if let Some(name) = pools.uuid_to_name.get(u) {
                info.insert("name".to_string(), Variant(Box::new(name.to_string())));
            }
            (uuid, info)
        })
        .collect::<HashMap<_, _>>()
}

/// Convert a stopped pool data structure to a property format.
pub fn stopped_pools_to_prop(pools: &StoppedPoolsInfo, metadata: bool) -> StoppedOrLockedPools {
    pools
        .stopped
        .iter()
        .chain(pools.partially_constructed.iter())
        .map(|(u, stopped)| {
            let mut map = HashMap::new();
            if let Some(enc_info) = stopped.info.as_ref() {
                map.insert(
                    "key_description".to_string(),
                    Variant(Box::new(result_option_to_tuple(
                        enc_info
                            .key_description()
                            .map(|opt| opt.map(|kd| kd.as_application_str().to_string())),
                        String::new(),
                    )) as Box<dyn RefArg>),
                );
                map.insert(
                    "clevis_info".to_string(),
                    Variant(Box::new(result_option_to_tuple(
                        enc_info
                            .clevis_info()
                            .map(|opt| opt.map(|(pin, cfg)| (pin.to_owned(), cfg.to_string()))),
                        (String::new(), String::new()),
                    )) as Box<dyn RefArg>),
                );
            }
            if let Some(name) = pools.uuid_to_name.get(u) {
                map.insert("name".to_string(), Variant(Box::new(name.to_string())));
            }
            map.insert(
                "devs".to_string(),
                Variant(Box::new(
                    stopped
                        .devices
                        .iter()
                        .map(|d| {
                            let mut map = HashMap::new();
                            map.insert("devnode".to_string(), d.devnode.display().to_string());
                            map.insert("uuid".to_string(), uuid_to_string!(d.uuid));
                            map
                        })
                        .collect::<Vec<_>>(),
                )),
            );
            if metadata {
                map.insert(
                    "metadata_version".to_string(),
                    match stopped.metadata_version {
                        Some(m) => Variant(Box::new((true, m as u64))),
                        None => Variant(Box::new((false, 0))),
                    },
                );
                map.insert(
                    "features".to_string(),
                    match stopped.features {
                        Some(ref f) => {
                            let mut feat = HashMap::new();
                            if f.encryption {
                                feat.insert("encryption".to_string(), true);
                            }
                            Variant(Box::new((true, feat)))
                        }
                        None => Variant(Box::new((false, HashMap::<String, bool>::new()))),
                    },
                );
            }
            (uuid_to_string!(u), map)
        })
        .collect::<HashMap<_, _>>()
}

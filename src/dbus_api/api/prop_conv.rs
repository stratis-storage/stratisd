// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use dbus::arg::{RefArg, Variant};

use crate::{
    dbus_api::util::result_option_to_tuple,
    engine::{LockedPoolInfo, PoolUuid},
};

/// D-Bus representation of locked pools.
pub type LockedPools = HashMap<String, HashMap<String, Variant<Box<dyn RefArg>>>>;

/// Convert a locked pool data structure to a property format.
pub fn locked_pools_to_prop(
    pools: HashMap<PoolUuid, LockedPoolInfo>,
) -> HashMap<String, HashMap<String, Variant<Box<dyn RefArg>>>> {
    pools
        .into_iter()
        .map(|(u, locked)| {
            (
                uuid_to_string!(u),
                vec![
                    (
                        "key_description".to_string(),
                        Variant(Box::new(result_option_to_tuple(
                            locked
                                .info
                                .key_description()
                                .map(|opt| opt.map(|kd| kd.as_application_str().to_string())),
                            String::new(),
                        )) as Box<dyn RefArg>),
                    ),
                    (
                        "clevis_info".to_string(),
                        Variant(Box::new(result_option_to_tuple(
                            locked
                                .info
                                .clevis_info()
                                .map(|opt| opt.map(|(pin, cfg)| (pin.to_owned(), cfg.to_string()))),
                            (String::new(), String::new()),
                        )) as Box<dyn RefArg>),
                    ),
                    (
                        "devs".to_string(),
                        Variant(Box::new(
                            locked
                                .devices
                                .into_iter()
                                .map(|d| {
                                    let mut map = HashMap::new();
                                    map.insert(
                                        "devnode".to_string(),
                                        d.devnode.display().to_string(),
                                    );
                                    map.insert("uuid".to_string(), uuid_to_string!(d.uuid));
                                    map
                                })
                                .collect::<Vec<_>>(),
                        )),
                    ),
                ]
                .into_iter()
                .collect::<HashMap<_, _>>(),
            )
        })
        .collect()
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use dbus::{
    arg::{RefArg, Variant},
    Message,
};
use dbus_tree::{MTSync, MethodInfo, MethodResult};
use itertools::Itertools;

use crate::dbus_api::{
    api::shared::list_keys,
    consts,
    types::TData,
    util::{result_option_to_tuple, result_to_tuple},
};

const ALL_PROPERTIES: [&str; 2] = [consts::KEY_LIST_PROP, consts::LOCKED_POOL_DEVS];

type LockedPoolsWithDevs = HashMap<String, HashMap<String, Variant<Box<dyn RefArg>>>>;

pub fn locked_pools_with_devs(
    info: &MethodInfo<MTSync<TData>, TData>,
) -> Result<LockedPoolsWithDevs, String> {
    let dbus_context = info.tree.get_data();

    let engine = dbus_context.engine.blocking_lock();
    Ok(engine
        .locked_pools()
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
        .collect())
}

#[allow(clippy::unnecessary_wraps)]
fn get_properties_shared(
    m: &MethodInfo<MTSync<TData>, TData>,
    properties: &mut dyn Iterator<Item = String>,
) -> MethodResult {
    let message: &Message = m.msg;

    let return_message = message.method_return();

    let return_value: HashMap<String, (bool, Variant<Box<dyn RefArg>>)> = properties
        .unique()
        .filter_map(|prop| match prop.as_str() {
            consts::KEY_LIST_PROP => Some((prop, result_to_tuple(list_keys(m)))),
            consts::LOCKED_POOL_DEVS => Some((prop, result_to_tuple(locked_pools_with_devs(m)))),
            _ => None,
        })
        .collect();

    Ok(vec![return_message.append1(return_value)])
}

properties_footer!();

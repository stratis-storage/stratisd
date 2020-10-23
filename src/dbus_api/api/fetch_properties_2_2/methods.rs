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
    api::shared::{list_keys, locked_pool_uuids, locked_pools},
    consts,
    types::TData,
    util::result_to_tuple,
};

const ALL_PROPERTIES: [&str; 3] = [
    consts::KEY_LIST_PROP,
    consts::LOCKED_POOLS,
    consts::LOCKED_POOL_UUIDS,
];

#[allow(clippy::unknown_clippy_lints)]
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
            consts::LOCKED_POOLS => Some((prop, result_to_tuple(locked_pools(m)))),
            consts::LOCKED_POOL_UUIDS => Some((prop, result_to_tuple(locked_pool_uuids(m)))),
            _ => None,
        })
        .collect();

    Ok(vec![return_message.append1(return_value)])
}

properties_footer!();

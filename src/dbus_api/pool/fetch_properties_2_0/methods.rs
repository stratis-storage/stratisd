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
    consts,
    pool::shared::{get_pool_total_size, get_pool_total_used},
    types::TData,
    util::result_to_tuple,
};

const ALL_PROPERTIES: [&str; 2] = [consts::POOL_TOTAL_SIZE_PROP, consts::POOL_TOTAL_USED_PROP];

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
            consts::POOL_TOTAL_SIZE_PROP => Some((prop, result_to_tuple(get_pool_total_size(m)))),
            consts::POOL_TOTAL_USED_PROP => Some((prop, result_to_tuple(get_pool_total_used(m)))),
            _ => None,
        })
        .collect();

    Ok(vec![return_message.append1(return_value)])
}

properties_footer!();
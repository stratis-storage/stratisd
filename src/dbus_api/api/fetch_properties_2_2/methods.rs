// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use dbus::{
    arg::{RefArg, Variant},
    tree::{MTFn, MethodInfo, MethodResult},
    Message,
};
use itertools::Itertools;

use crate::dbus_api::{api::shared::list_keys, consts, types::TData, util::result_to_tuple};

const ALL_PROPERTIES: [&str; 2] = [consts::KEY_LIST_PROP, consts::LOCKED_POOLS];

pub fn locked_pools(
    info: &MethodInfo<MTFn<TData>, TData>,
) -> Result<HashMap<String, String>, String> {
    let dbus_context = info.tree.get_data();

    let engine = dbus_context.engine.borrow();
    Ok(engine
        .locked_pools()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(u, kd)| {
            (
                u.to_simple_ref().to_string(),
                kd.as_application_str().to_string(),
            )
        })
        .collect())
}

fn get_properties_shared(
    m: &MethodInfo<MTFn<TData>, TData>,
    properties: &mut dyn Iterator<Item = String>,
) -> MethodResult {
    let message: &Message = m.msg;

    let return_message = message.method_return();

    let return_value: HashMap<String, (bool, Variant<Box<dyn RefArg>>)> = properties
        .unique()
        .filter_map(|prop| match prop.as_str() {
            consts::KEY_LIST_PROP => Some((prop, result_to_tuple(list_keys(m)))),
            consts::LOCKED_POOLS => Some((prop, result_to_tuple(locked_pools(m)))),
            _ => None,
        })
        .collect();

    Ok(vec![return_message.append1(return_value)])
}

properties_footer!();

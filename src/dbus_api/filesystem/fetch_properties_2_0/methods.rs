// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use dbus::{
    self,
    arg::{RefArg, Variant},
    tree::{MTFn, MethodInfo, MethodResult},
    Message,
};
use itertools::Itertools;

use crate::dbus_api::{
    consts, filesystem::shared::filesystem_operation, types::TData, util::result_to_tuple,
};

const ALL_PROPERTIES: [&str; 1] = [consts::FILESYSTEM_USED_PROP];

fn get_properties_shared(
    m: &MethodInfo<MTFn<TData>, TData>,
    properties: &mut dyn Iterator<Item = String>,
) -> MethodResult {
    let message: &Message = m.msg;
    let object_path = &m.path;

    let return_message = message.method_return();

    let return_value: HashMap<String, (bool, Variant<Box<dyn RefArg>>)> = properties
        .unique()
        .filter_map(|prop| match prop.as_str() {
            consts::FILESYSTEM_USED_PROP => Some((
                prop,
                filesystem_operation(m.tree, object_path.get_name(), |(_, _, fs)| {
                    fs.used()
                        .map(|u| (*u).to_string())
                        .map_err(|e| e.to_string())
                }),
            )),
            _ => None,
        })
        .map(|(key, result)| result_to_tuple(key, result))
        .collect();

    Ok(vec![return_message.append1(return_value)])
}

properties_footer!();

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use dbus::{
    self,
    arg::{Array, RefArg, Variant},
    tree::{MTFn, MethodInfo, MethodResult},
    Message,
};
use itertools::Itertools;

use crate::dbus_api::{
    blockdev::shared::blockdev_operation,
    consts,
    types::TData,
    util::{get_next_arg, result_to_tuple},
};

pub fn get_all_properties(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    get_properties_shared(
        m,
        &mut vec![consts::BLOCKDEV_TOTAL_SIZE_PROP]
            .into_iter()
            .map(|s| s.to_string()),
    )
}

pub fn get_properties(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();
    let mut properties: Array<String, _> = get_next_arg(&mut iter, 0)?;
    get_properties_shared(m, &mut properties)
}

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
            consts::BLOCKDEV_TOTAL_SIZE_PROP => Some((
                prop,
                blockdev_operation(m.tree, object_path.get_name(), |_, bd| {
                    Ok((u128::from(*bd.size()) * devicemapper::SECTOR_SIZE as u128).to_string())
                }),
            )),
            _ => None,
        })
        .map(|(key, result)| result_to_tuple(key, result))
        .collect();

    Ok(vec![return_message.append1(return_value)])
}

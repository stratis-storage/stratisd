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

use crate::dbus_api::{consts, pool::shared::pool_operation, types::TData, util::result_to_tuple};

const ALL_PROPERTIES: [&str; 2] = [consts::POOL_TOTAL_SIZE_PROP, consts::POOL_TOTAL_USED_PROP];

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
            consts::POOL_TOTAL_SIZE_PROP => Some((
                prop,
                pool_operation(m.tree, object_path.get_name(), |(_, _, pool)| {
                    Ok((u128::from(*pool.total_physical_size())
                        * devicemapper::SECTOR_SIZE as u128)
                        .to_string())
                }),
            )),
            consts::POOL_TOTAL_USED_PROP => Some((
                prop,
                pool_operation(m.tree, object_path.get_name(), |(_, _, pool)| {
                    pool.total_physical_used()
                        .map_err(|e| e.to_string())
                        .map(|size| {
                            (u128::from(*size) * devicemapper::SECTOR_SIZE as u128).to_string()
                        })
                }),
            )),
            _ => None,
        })
        .map(|(key, result)| result_to_tuple(key, result))
        .collect();

    Ok(vec![return_message.append1(return_value)])
}

properties_footer!();
